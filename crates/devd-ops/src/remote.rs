//! Deploying a site to a server: link it, build, ship, switch, roll back; set a fresh server up.
//!
//! Two modes, chosen per site-and-server pair:
//!
//! * `releases` (the default): devd builds locally, ships the project over SSH as one archive, unpacks it into
//!   `releases/<time>`, links the shared `.env` and `storage`, runs the steps on the server, and only then points
//!   `current` at the new release with one atomic rename. A failure before that leaves the running site untouched.
//!   Rollback is pointing `current` back. The web server's root is `current/<web dir>`.
//! * `git`: for servers a panel such as Ploi or Forge manages. The server pulls the branch and runs the site's deploy
//!   script exactly as written, `cd`, `sudo` and PHP-FPM reloads included.
//!
//! Nothing here is clever with secrets: the `.env` is never in the archive, and `remote.env_push` is a separate act.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};

use devd_core::capability::Ctx;
use devd_core::store_ops::OpsItem;
use devd_core::{util, DevdError, Result};

use crate::servers::{self, shq, Server};

/// The kind under which links are stored.
pub const KIND: &str = "remote";

/// A site linked to a place on a server.
#[derive(Debug, Clone, Serialize)]
pub struct Link {
    /// `<site>@<server>`.
    pub id: String,
    /// The local site.
    pub site: String,
    /// The server.
    pub server: String,
    /// The folder on the server.
    pub path: String,
    /// `releases` or `git`.
    pub mode: String,
    /// The branch (git mode, and what a staging copy follows).
    pub branch: String,
    /// The web root inside a release, e.g. `public`.
    pub web_dir: String,
    /// The PHP version on the server, e.g. `8.4`.
    pub php: String,
    /// How many releases to keep.
    pub keep: u32,
    /// The domain the site answers on.
    pub domain: Option<String>,
    /// Local commands run before packaging (`npm ci`, `npm run build`).
    pub build: Vec<String>,
    /// Ignored paths that still ship (`public/build`).
    pub include: Vec<String>,
    /// Commands run on the server inside the new release; empty means the default for the project.
    pub steps: Vec<String>,
}

/// A path on the server that a deploy may own: absolute, safe characters, at least two components, not a system folder.
pub fn valid_remote_path(p: &str) -> bool {
    if !p.starts_with('/') || p.len() > 200 || p.contains("..") || p.contains("//") || p.ends_with('/') && p.len() > 1 {
        return false;
    }
    if !p.chars().all(|c| c.is_ascii_alphanumeric() || "/_.-".contains(c)) {
        return false;
    }
    let parts: Vec<&str> = p.split('/').filter(|s| !s.is_empty()).collect();
    parts.len() >= 2 && !["bin", "boot", "dev", "etc", "lib", "lib64", "proc", "root", "run", "sbin", "sys", "usr"].contains(&parts[0])
}

fn valid_rel(p: &str) -> bool {
    !p.is_empty() && p.len() <= 200 && !p.starts_with('/') && !p.contains("..") && p.chars().all(|c| c.is_ascii_alphanumeric() || "/_.-".contains(c))
}

fn valid_php(v: &str) -> bool {
    let mut it = v.split('.');
    matches!((it.next(), it.next(), it.next()), (Some(a), Some(b), None) if a.len() == 1 && (1..=2).contains(&b.len()) && a.chars().all(|c| c.is_ascii_digit()) && b.chars().all(|c| c.is_ascii_digit()))
}

fn link_of(item: &OpsItem) -> Link {
    let d = &item.data;
    let s = |k: &str| d[k].as_str().unwrap_or("").to_string();
    let v = |k: &str| d[k].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
    Link {
        id: item.id.clone(),
        site: s("site"),
        server: s("server"),
        path: s("path"),
        mode: s("mode"),
        branch: s("branch"),
        web_dir: s("web_dir"),
        php: s("php"),
        keep: d["keep"].as_u64().unwrap_or(5) as u32,
        domain: d["domain"].as_str().map(str::to_string),
        build: v("build"),
        include: v("include"),
        steps: v("steps"),
    }
}

/// The links, optionally one site's.
pub fn links(ctx: &Ctx, site: Option<&str>) -> Result<Vec<Link>> {
    Ok(ctx.store.ops_list(KIND, site)?.iter().map(link_of).collect())
}

/// One link, by id or by site name when the site has only one.
pub fn link(ctx: &Ctx, id: &str) -> Result<Link> {
    if let Some(i) = ctx.store.ops_get(KIND, id)? {
        return Ok(link_of(&i));
    }
    let by_site: Vec<Link> = links(ctx, Some(id))?;
    match by_site.len() {
        1 => Ok(by_site.into_iter().next().unwrap_or_else(|| unreachable!())),
        0 => Err(DevdError::not_found(format!("`{id}` is not a link; `remote.link` links a site to a server (ids look like `shop@hetzner`)"))),
        _ => Err(DevdError::invalid(format!("{id} is linked to several servers ({}); name one as `{id}@<server>`", by_site.iter().map(|l| l.id.clone()).collect::<Vec<_>>().join(", ")))),
    }
}

/// What a link is made of, before it is checked.
#[derive(Debug, Clone, Default)]
pub struct LinkSpec {
    /// The local site.
    pub site: String,
    /// The server.
    pub server: String,
    /// The folder on the server.
    pub path: String,
    /// `releases` or `git`. Default `releases`.
    pub mode: Option<String>,
    /// The branch. Default: the repository's current branch, else `main`.
    pub branch: Option<String>,
    /// The web root inside a release. Default `public`.
    pub web_dir: Option<String>,
    /// The server's PHP, e.g. `8.4`. Default `8.4`.
    pub php: Option<String>,
    /// Releases to keep. Default 5.
    pub keep: Option<u32>,
    /// The domain.
    pub domain: Option<String>,
    /// Local build commands. Default for a project with a `package.json` and a `build` script: `npm ci`, `npm run build`.
    pub build: Option<Vec<String>>,
    /// Ignored paths to ship. Default `public/build` for a project that builds.
    pub include: Option<Vec<String>>,
    /// Server steps. Default by project type.
    pub steps: Option<Vec<String>>,
}

/// The default local build for a project folder.
fn default_build(root: &Path) -> (Vec<String>, Vec<String>) {
    let pkg = std::fs::read_to_string(root.join("package.json")).unwrap_or_default();
    if pkg.contains("\"build\"") {
        let install = if root.join("package-lock.json").is_file() { "npm ci" } else { "npm install" };
        return (vec![install.to_string(), "npm run build".to_string()], vec!["public/build".to_string()]);
    }
    (Vec::new(), Vec::new())
}

/// The steps run on the server inside a new release, by project type.
pub fn default_steps(root: &Path) -> Vec<String> {
    let mut v = Vec::new();
    if root.join("composer.json").is_file() {
        v.push("composer install --no-dev --prefer-dist --optimize-autoloader --no-interaction".to_string());
    }
    if root.join("artisan").is_file() {
        v.extend(["php artisan migrate --force", "php artisan config:cache", "php artisan route:cache", "php artisan view:cache"].map(String::from));
    }
    v
}

/// Link a site to a place on a server.
pub fn make_link(ctx: &Ctx, spec: &LinkSpec) -> Result<Link> {
    let (site, root) = crate::site_root(ctx, &spec.site)?;
    let srv = servers::get(ctx, &spec.server)?;
    if !valid_remote_path(&spec.path) {
        return Err(DevdError::invalid("the path on the server is absolute, at least two folders deep (`/var/www/shop`), with letters, digits and `/ _ . -`, and not a system folder"));
    }
    let mode = spec.mode.clone().unwrap_or_else(|| "releases".to_string());
    if mode != "releases" && mode != "git" {
        return Err(DevdError::invalid("the mode is `releases` (atomic releases) or `git` (the server pulls and runs the deploy script)"));
    }
    let web_dir = spec.web_dir.clone().unwrap_or_else(|| "public".to_string());
    if !valid_rel(&web_dir) {
        return Err(DevdError::invalid("the web directory is a relative folder such as `public`"));
    }
    let php = spec.php.clone().unwrap_or_else(|| "8.4".to_string());
    if !valid_php(&php) {
        return Err(DevdError::invalid("the PHP version looks like `8.4`"));
    }
    let keep = spec.keep.unwrap_or(5);
    if !(2..=30).contains(&keep) {
        return Err(DevdError::invalid("keep between 2 and 30 releases"));
    }
    if let Some(d) = &spec.domain {
        if !servers::valid_host(d) || d.contains(':') {
            return Err(DevdError::invalid("that is not a domain name"));
        }
    }
    let (b, i) = default_build(&root);
    let build = spec.build.clone().unwrap_or(b);
    let include = spec.include.clone().unwrap_or(i);
    let steps = spec.steps.clone().unwrap_or_default();
    for c in build.iter().chain(steps.iter()) {
        if c.is_empty() || c.len() > 500 || c.contains(['\0', '\n', '\r']) {
            return Err(DevdError::invalid("each command is one line of up to 500 characters"));
        }
    }
    if include.iter().any(|p| !valid_rel(p)) {
        return Err(DevdError::invalid("each shipped path is relative, e.g. `public/build`"));
    }
    let branch = spec.branch.clone().unwrap_or_else(|| if root.join(".git").exists() { crate::deploy::current_branch(ctx, &root) } else { "main".to_string() });
    let id = format!("{}@{}", site.name, srv.name);
    ctx.store.ops_put(
        KIND,
        &id,
        &site.name,
        &json!({ "site": site.name, "server": srv.name, "path": spec.path, "mode": mode, "branch": branch, "web_dir": web_dir, "php": php, "keep": keep, "domain": spec.domain, "build": build, "include": include, "steps": steps }),
    )?;
    link(ctx, &id)
}

/// Remove a link. The server is not touched.
pub fn unlink(ctx: &Ctx, id: &str) -> Result<()> {
    let l = link(ctx, id)?;
    ctx.store.ops_delete(KIND, &l.id)?;
    Ok(())
}

/// Change the steps run on the server in a new release.
pub fn set_steps(ctx: &Ctx, id: &str, steps: &[String]) -> Result<Link> {
    let l = link(ctx, id)?;
    for c in steps {
        if c.is_empty() || c.len() > 500 || c.contains(['\0', '\n', '\r']) {
            return Err(DevdError::invalid("each step is one line of up to 500 characters"));
        }
    }
    let mut item = ctx.store.ops_get(KIND, &l.id)?.ok_or_else(|| DevdError::internal("the link vanished"))?;
    item.data["steps"] = json!(steps);
    ctx.store.ops_put(KIND, &l.id, &item.site, &item.data)?;
    link(ctx, &l.id)
}

// ------------------------------------------------------------------------------------ packaging

/// The files to ship, relative to the project: what Git tracks or would track, plus the included ignored paths,
/// without `.git`, dependencies that are installed on the server, logs, and every `.env`.
pub fn file_list(root: &Path, include: &[String]) -> Result<Vec<String>> {
    let mut files: Vec<String> = Vec::new();
    let git = root.join(".git").exists();
    if git {
        let out = std::process::Command::new("git").args(["ls-files", "-z", "--cached", "--others", "--exclude-standard"]).current_dir(root).output();
        if let Ok(o) = out {
            if o.status.success() {
                files = String::from_utf8_lossy(&o.stdout).split('\0').filter(|s| !s.is_empty()).map(str::to_string).collect();
            }
        }
    }
    if files.is_empty() {
        walk(root, root, &mut files, 0);
    }
    for inc in include {
        let dir = root.join(inc);
        if dir.is_dir() {
            walk(root, &dir, &mut files, 0);
        } else if dir.is_file() {
            files.push(inc.clone());
        }
    }
    files.retain(|f| {
        let f = f.replace('\\', "/");
        !(f.starts_with(".git/") || f.starts_with("node_modules/") || f.starts_with("vendor/") || f.starts_with("storage/logs/") || f == ".env" || f.starts_with(".env.") || f.ends_with("/.env") || f.starts_with(".devd") || f.contains("/node_modules/") || f.ends_with(".sqlite") || f.ends_with(".sqlite-journal"))
    });
    files.sort();
    files.dedup();
    files.retain(|f| !f.contains(['\n', '\r']) && root.join(f).is_file());
    Ok(files)
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>, depth: usize) {
    if depth > 24 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if p.is_dir() {
            if name == ".git" || name == "node_modules" || name == "vendor" {
                continue;
            }
            walk(root, &p, out, depth + 1);
        } else if let Ok(rel) = p.strip_prefix(root) {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

/// Pack the files into a gzip tar with the system's `tar`, which Windows 10 and later include.
pub fn package(ctx: &Ctx, root: &Path, files: &[String], id: &str) -> Result<PathBuf> {
    let dir = ctx.paths.cache();
    std::fs::create_dir_all(&dir)?;
    let list = dir.join(format!("deploy-{id}.list"));
    let archive = dir.join(format!("deploy-{id}.tgz"));
    std::fs::write(&list, files.join("\n"))?;
    let out = util::run(Path::new("tar"), &["-czf".into(), archive.display().to_string().replace('\\', "/"), "-T".into(), list.display().to_string().replace('\\', "/")], Some(root), &[], Some(Duration::from_secs(900)));
    let _ = std::fs::remove_file(&list);
    let out = out.map_err(|e| DevdError::unsupported(format!("could not run tar: {e}")))?;
    if out.code != 0 {
        let _ = std::fs::remove_file(&archive);
        return Err(DevdError::unsupported(format!("tar failed: {}", out.stderr.trim())));
    }
    Ok(archive)
}

// -------------------------------------------------------------------------------------- scripts

/// The service name of the server's PHP-FPM.
fn fpm(l: &Link) -> String {
    format!("php{}-fpm", l.php)
}

/// The shell script that unpacks an uploaded archive into a new release and switches to it. `ts` names the release.
pub fn release_script(l: &Link, ts: &str, steps: &[String], commit: &str) -> String {
    let p = &l.path;
    let mut s = String::new();
    s.push_str("set -e\n");
    s.push_str(&format!("cd {}\n", shq(p)));
    s.push_str(&format!("R={}\n", shq(&format!("releases/{ts}"))));
    s.push_str("mkdir -p \"$R\"\n");
    s.push_str(&format!("tar -xzf {} -C \"$R\"\n", shq(&format!("releases/{ts}.tgz"))));
    s.push_str(&format!("rm -f {}\n", shq(&format!("releases/{ts}.tgz"))));
    s.push_str("mkdir -p shared/storage/app/public shared/storage/framework/cache shared/storage/framework/sessions shared/storage/framework/views shared/storage/logs\n");
    s.push_str("if [ ! -f shared/.env ]; then echo \"devd: shared/.env is missing on the server; run remote.env_push or put one at \"$(pwd)\"/shared/.env\" >&2; rm -rf \"$R\"; exit 3; fi\n");
    s.push_str("rm -rf \"$R/storage\"\n");
    s.push_str("ln -s \"$(pwd)/shared/storage\" \"$R/storage\"\n");
    s.push_str("ln -sf \"$(pwd)/shared/.env\" \"$R/.env\"\n");
    s.push_str(&format!("printf '%s\\n' {} > \"$R/.devd-commit\"\n", shq(commit)));
    s.push_str("cd \"$R\"\n");
    for st in steps {
        s.push_str(&format!("echo {}\n{}\n", shq(&format!("$ {st}")), st));
    }
    s.push_str(&format!("cd {}\n", shq(p)));
    s.push_str("ln -sfn \"$(pwd)/$R\" current.new\nmv -Tf current.new current\n");
    s.push_str(&format!("if command -v sudo >/dev/null 2>&1; then sudo -n service {} reload >/dev/null 2>&1 || true; fi\n", fpm(l)));
    s.push_str("(cd current && php artisan queue:restart >/dev/null 2>&1 || true)\n");
    s.push_str(&format!("ls -1dt releases/*/ 2>/dev/null | tail -n +{} | while read old; do rm -rf \"$old\"; done\n", l.keep + 1));
    s.push_str("echo \"devd: current -> $R\"\n");
    s
}

/// The shell script of a git-mode deploy: pull the branch, then the site's deploy script as written.
pub fn git_script(l: &Link, script_lines: &[String]) -> String {
    let mut s = String::from("set -e\n");
    s.push_str(&format!("cd {}\n", shq(&l.path)));
    s.push_str(&format!("git fetch --all --prune\ngit checkout {}\ngit pull --ff-only origin {}\n", shq(&l.branch), shq(&l.branch)));
    for line in script_lines {
        s.push_str(&format!("echo {}\n{}\n", shq(&format!("$ {line}")), line));
    }
    s
}

/// The script that points `current` at an earlier release.
pub fn rollback_script(l: &Link, to: Option<&str>) -> String {
    let mut s = String::from("set -e\n");
    s.push_str(&format!("cd {}\n", shq(&l.path)));
    s.push_str("cur=$(basename \"$(readlink current)\")\n");
    match to {
        Some(t) => s.push_str(&format!("target={}\n[ -d \"releases/$target\" ] || {{ echo \"devd: there is no release $target\" >&2; exit 4; }}\n", shq(t))),
        None => s.push_str("target=$(ls -1 releases | sort | grep -B1 -x \"$cur\" | head -n1)\n[ -n \"$target\" ] && [ \"$target\" != \"$cur\" ] || { echo \"devd: there is no earlier release to go back to\" >&2; exit 4; }\n"),
    }
    s.push_str("ln -sfn \"$(pwd)/releases/$target\" current.new\nmv -Tf current.new current\n");
    s.push_str(&format!("if command -v sudo >/dev/null 2>&1; then sudo -n service {} reload >/dev/null 2>&1 || true; fi\n", fpm(l)));
    s.push_str("(cd current && php artisan queue:restart >/dev/null 2>&1 || true)\n");
    s.push_str("echo \"devd: current -> releases/$target (was $cur)\"\n");
    s
}

/// The nginx server block for a site: HTTP, the web root inside `current`, PHP-FPM, no dotfiles.
pub fn nginx_vhost(l: &Link) -> Result<String> {
    let domain = l.domain.as_deref().ok_or_else(|| DevdError::invalid("link the site with a domain first (`remote.link` with `domain`)"))?;
    let root = if l.mode == "releases" { format!("{}/current/{}", l.path, l.web_dir) } else { format!("{}/{}", l.path, l.web_dir) };
    Ok(format!(
        "server {{\n    listen 80;\n    listen [::]:80;\n    server_name {domain};\n    root {root};\n\n    index index.php index.html;\n    charset utf-8;\n    client_max_body_size 64m;\n\n    location / {{\n        try_files $uri $uri/ /index.php?$query_string;\n    }}\n\n    location ~ \\.php$ {{\n        try_files $uri =404;\n        fastcgi_pass unix:/run/php/php{php}-fpm.sock;\n        fastcgi_index index.php;\n        include fastcgi_params;\n        fastcgi_param SCRIPT_FILENAME $realpath_root$fastcgi_script_name;\n        fastcgi_param DOCUMENT_ROOT $realpath_root;\n    }}\n\n    location ~ /\\.(?!well-known).* {{\n        deny all;\n    }}\n}}\n",
        php = l.php
    ))
}

/// What `server.provision` installs.
#[derive(Debug, Clone)]
pub struct Stack {
    /// PHP version, e.g. `8.4`.
    pub php: String,
    /// `mariadb`, `mysql` or none.
    pub database: Option<String>,
    /// Redis.
    pub redis: bool,
    /// Node.js (the NodeSource LTS).
    pub node: bool,
    /// supervisor, for queue workers.
    pub supervisor: bool,
}

impl Default for Stack {
    fn default() -> Self {
        Self { php: "8.4".into(), database: Some("mariadb".into()), redis: false, node: true, supervisor: true }
    }
}

/// The script that sets up a fresh Ubuntu or Debian server for PHP sites. It is plain apt and the vendors' own
/// repositories, it is idempotent, it is shown in the plan before anything runs, and it never touches a site's files.
pub fn provision_script(stack: &Stack) -> Result<String> {
    if !valid_php(&stack.php) {
        return Err(DevdError::invalid("the PHP version looks like `8.4`"));
    }
    if let Some(d) = &stack.database {
        if d != "mariadb" && d != "mysql" {
            return Err(DevdError::invalid("the database is `mariadb`, `mysql` or none"));
        }
    }
    let v = &stack.php;
    let exts = ["fpm", "cli", "common", "mysql", "sqlite3", "curl", "mbstring", "xml", "zip", "gd", "intl", "bcmath", "opcache"].map(|e| format!("php{v}-{e}")).join(" ");
    let mut s = String::from("set -eu\nexport DEBIAN_FRONTEND=noninteractive\n");
    s.push_str("if [ \"$(id -u)\" -ne 0 ]; then echo \"devd: the provision script must run as root (use root, or a user with passwordless sudo)\" >&2; exit 2; fi\n");
    s.push_str(". /etc/os-release\ncase \"$ID\" in ubuntu|debian) ;; *) echo \"devd: this script supports Ubuntu and Debian, not $ID\" >&2; exit 2;; esac\n");
    s.push_str("apt-get update -y\napt-get install -y ca-certificates curl gnupg lsb-release unzip git ufw nginx software-properties-common apt-transport-https\n");
    s.push_str("if [ \"$ID\" = ubuntu ]; then add-apt-repository -y ppa:ondrej/php; else curl -fsSL https://packages.sury.org/php/apt.gpg -o /etc/apt/trusted.gpg.d/php.gpg && echo \"deb https://packages.sury.org/php/ $(lsb_release -sc) main\" > /etc/apt/sources.list.d/php.list; fi\n");
    s.push_str("apt-get update -y\n");
    s.push_str(&format!("apt-get install -y {exts}\n"));
    s.push_str("if ! command -v composer >/dev/null 2>&1; then\n  EXPECTED=$(curl -fsSL https://composer.github.io/installer.sig)\n  php -r \"copy('https://getcomposer.org/installer', '/tmp/composer-setup.php');\"\n  ACTUAL=$(php -r \"echo hash_file('sha384', '/tmp/composer-setup.php');\")\n  [ \"$EXPECTED\" = \"$ACTUAL\" ] || { echo 'devd: the Composer installer failed its checksum' >&2; rm -f /tmp/composer-setup.php; exit 3; }\n  php /tmp/composer-setup.php --install-dir=/usr/local/bin --filename=composer && rm -f /tmp/composer-setup.php\nfi\n");
    if stack.node {
        s.push_str("if ! command -v node >/dev/null 2>&1; then curl -fsSL https://deb.nodesource.com/setup_22.x | bash - && apt-get install -y nodejs; fi\n");
    }
    match stack.database.as_deref() {
        Some("mariadb") => s.push_str("apt-get install -y mariadb-server\nsystemctl enable --now mariadb\n"),
        Some("mysql") => s.push_str("apt-get install -y mysql-server\nsystemctl enable --now mysql\n"),
        _ => {}
    }
    if stack.redis {
        s.push_str("apt-get install -y redis-server\nsystemctl enable --now redis-server\n");
    }
    if stack.supervisor {
        s.push_str("apt-get install -y supervisor\nsystemctl enable --now supervisor\n");
    }
    s.push_str("apt-get install -y certbot python3-certbot-nginx\n");
    s.push_str("ufw allow OpenSSH\nufw allow 'Nginx Full'\nufw --force enable\n");
    s.push_str(&format!("systemctl enable --now nginx php{v}-fpm\n"));
    s.push_str("echo devd-provisioned\n");
    Ok(s)
}

// ------------------------------------------------------------------------------------- running

fn sudo_wrap(srv: &Server, script_cmd: &str) -> String {
    if srv.user == "root" {
        script_cmd.to_string()
    } else {
        format!("sudo -n {script_cmd}")
    }
}

/// What a remote action did.
#[derive(Debug, Clone, Serialize)]
pub struct Done {
    /// What ran, in a line.
    pub what: String,
    /// What it printed.
    pub output: String,
    /// Seconds.
    pub seconds: f64,
}

fn must(r: servers::Remote, what: &str) -> Result<Done> {
    if r.code != 0 {
        let tail: String = format!("{}{}", r.stdout, r.stderr).lines().rev().take(12).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
        return Err(DevdError::unsupported(format!("{what} failed (exit {}):\n{tail}", r.code)));
    }
    Ok(Done { what: what.to_string(), output: format!("{}{}", r.stdout, r.stderr), seconds: r.seconds })
}

/// Set a fresh server up for PHP sites.
pub fn provision(ctx: &Ctx, server: &str, stack: &Stack) -> Result<Done> {
    let srv = servers::get(ctx, server)?;
    let script = provision_script(stack)?;
    let file = ctx.paths.cache().join(format!("provision-{}.sh", util::new_id()));
    std::fs::create_dir_all(ctx.paths.cache())?;
    std::fs::write(&file, &script)?;
    let r = servers::run(ctx, &srv, &sudo_wrap(&srv, "bash -s"), Some(&file), Duration::from_secs(2400), false);
    let _ = std::fs::remove_file(&file);
    must(r?, &format!("provisioning {server}"))
}

/// The releases on the server, oldest first, and which one is current.
pub fn releases(ctx: &Ctx, id: &str) -> Result<Value> {
    let l = link(ctx, id)?;
    let srv = servers::get(ctx, &l.server)?;
    let cmd = format!("cd {} 2>/dev/null && {{ echo current=$(basename \"$(readlink current 2>/dev/null)\"); for r in $(ls -1 releases 2>/dev/null | sort); do echo \"release=$r $(cat releases/$r/.devd-commit 2>/dev/null)\"; done; }} || echo missing=1", shq(&l.path));
    let r = servers::run(ctx, &srv, &cmd, None, Duration::from_secs(60), false)?;
    if r.code != 0 {
        return Err(DevdError::unsupported(format!("could not read the releases: {}", r.stderr.trim())));
    }
    let mut current = String::new();
    let mut list = Vec::new();
    for line in r.stdout.lines() {
        if let Some(c) = line.strip_prefix("current=") {
            current = c.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("release=") {
            let mut p = rest.splitn(2, ' ');
            let name = p.next().unwrap_or("").to_string();
            list.push(json!({ "release": name, "commit": p.next().unwrap_or("").trim(), "current": name == current }));
        }
    }
    Ok(json!({ "link": l.id, "path": l.path, "current": current, "releases": list, "folder_exists": !r.stdout.contains("missing=1") }))
}

/// Point `current` at an earlier release.
pub fn rollback(ctx: &Ctx, id: &str, to: Option<&str>) -> Result<Done> {
    let l = link(ctx, id)?;
    if l.mode != "releases" {
        return Err(DevdError::invalid("rollback is for the `releases` mode; in `git` mode roll back with `deploy.rollback` on the commit"));
    }
    if let Some(t) = to {
        if !valid_rel(t) || t.contains('/') {
            return Err(DevdError::invalid("a release is named like `20261023-153000`"));
        }
    }
    let srv = servers::get(ctx, &l.server)?;
    let r = servers::run(ctx, &srv, &format!("bash -c {}", shq(&rollback_script(&l, to))), None, Duration::from_secs(120), false)?;
    must(r, &format!("rolling {} back on {}", l.site, l.server))
}

/// Put the site's `.env` on the server as `shared/.env`, set for production. Refuses to replace one that is there.
pub fn env_push(ctx: &Ctx, id: &str, overwrite: bool) -> Result<Value> {
    let l = link(ctx, id)?;
    let (_, root) = crate::site_root(ctx, &l.site)?;
    let text = std::fs::read_to_string(crate::env::path_of(&root)).map_err(|_| DevdError::not_found(format!("{} has no .env to send", l.site)))?;
    let mut set = std::collections::BTreeMap::from([("APP_ENV".to_string(), "production".to_string()), ("APP_DEBUG".to_string(), "false".to_string())]);
    if let Some(d) = &l.domain {
        set.insert("APP_URL".to_string(), format!("https://{d}"));
    }
    let adjusted = crate::env::with_set(&text, &set)?;
    let srv = servers::get(ctx, &l.server)?;
    let check = servers::run(ctx, &srv, &format!("test -f {}/shared/.env && echo present || echo absent", shq(&l.path)), None, Duration::from_secs(45), false)?;
    if check.stdout.contains("present") && !overwrite {
        return Err(DevdError::conflict(format!("{}:{}/shared/.env already exists; pass overwrite to replace it", l.server, l.path)));
    }
    let tmp = ctx.paths.cache().join(format!("env-{}.tmp", util::new_id()));
    std::fs::create_dir_all(ctx.paths.cache())?;
    std::fs::write(&tmp, adjusted)?;
    let r = servers::run(ctx, &srv, &format!("mkdir -p {p}/shared && umask 077 && cat > {p}/shared/.env", p = shq(&l.path)), Some(&tmp), Duration::from_secs(60), false);
    let _ = std::fs::remove_file(&tmp);
    must(r?, "sending the .env")?;
    Ok(json!({ "link": l.id, "path": format!("{}/shared/.env", l.path), "set_for_production": set.keys().collect::<Vec<_>>(), "note": "The file is readable only by the server user. Review it there: database and mail settings are usually different in production." }))
}

/// Install the nginx server block for the site on the server and reload nginx.
pub fn site_create(ctx: &Ctx, id: &str) -> Result<Done> {
    let l = link(ctx, id)?;
    let conf = nginx_vhost(&l)?;
    let domain = l.domain.clone().unwrap_or_default();
    let srv = servers::get(ctx, &l.server)?;
    let tmp = ctx.paths.cache().join(format!("vhost-{}.conf", util::new_id()));
    std::fs::create_dir_all(ctx.paths.cache())?;
    std::fs::write(&tmp, conf)?;
    let target = format!("/etc/nginx/sites-available/{domain}");
    let enabled = format!("/etc/nginx/sites-enabled/{domain}");
    let script = format!(
        "set -e; {sudo}tee {t} >/dev/null; {sudo}ln -sfn {t} {e}; {sudo}nginx -t && {sudo}systemctl reload nginx; mkdir -p {p}/releases {p}/shared; echo devd-site-ok",
        sudo = if srv.user == "root" { "" } else { "sudo -n " },
        t = shq(&target),
        e = shq(&enabled),
        p = shq(&l.path)
    );
    let r = servers::run(ctx, &srv, &format!("bash -c {}", shq(&script)), Some(&tmp), Duration::from_secs(120), false);
    let _ = std::fs::remove_file(&tmp);
    must(r?, &format!("creating the site {domain} on {}", l.server))
}

/// Get an HTTPS certificate with Let's Encrypt (certbot) for the site's domain. The domain must already point at the server.
pub fn ssl(ctx: &Ctx, id: &str, email: &str) -> Result<Done> {
    let l = link(ctx, id)?;
    let domain = l.domain.clone().ok_or_else(|| DevdError::invalid("link the site with a domain first"))?;
    if !email.contains('@') || email.len() > 200 || email.contains(char::is_whitespace) || email.starts_with('-') || email.contains(['\'', '"', ';', '&', '|', '$', '`']) {
        return Err(DevdError::invalid("that is not a usable email address"));
    }
    let srv = servers::get(ctx, &l.server)?;
    let cmd = format!("{}certbot --nginx -d {} --non-interactive --agree-tos -m {} --redirect", if srv.user == "root" { "" } else { "sudo -n " }, shq(&domain), shq(email));
    let r = servers::run(ctx, &srv, &cmd, None, Duration::from_secs(300), false)?;
    must(r, &format!("getting a certificate for {domain}"))
}

// ------------------------------------------------------------------------------------ deploying

fn append(log: &Path, text: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log) {
        let _ = writeln!(f, "{text}");
    }
}

/// Start a deploy to a server and return at once with its deployment; the work runs on its own thread, and the
/// deployment is recorded, logged and announced like a local one.
pub fn deploy(ctx: &Ctx, id: &str, trigger: &str) -> Result<crate::deploy::Deployment> {
    let l = link(ctx, id)?;
    let srv = servers::get(ctx, &l.server)?;
    let (site, root) = crate::site_root(ctx, &l.site)?;
    if let Some(other) = crate::deploy::running_now(ctx, &l.site)? {
        return Err(DevdError::conflict(format!("deployment {other} of {} is still running", l.site)));
    }
    let dep = util::new_id()[..8].to_string();
    let started = util::now();
    let doc = json!({ "state": "running", "trigger": trigger, "started_at": started, "pid": std::process::id(), "target": format!("{}:{}", l.server, l.path), "mode": l.mode });
    ctx.store.ops_put(crate::deploy::DEPLOY, &dep, &l.site, &doc)?;
    let (ctx2, dep2) = (ctx.clone(), dep.clone());
    std::thread::spawn(move || {
        let ctx = ctx2;
        let log = crate::deploy::log_path(&ctx, &dep2);
        let _ = std::fs::write(&log, format!("deploy {dep2} of {} to {}:{} ({} mode) started {started}\n", l.site, l.server, l.path, l.mode));
        let t0 = std::time::Instant::now();
        let outcome = run_deploy(&ctx, &l, &srv, &site, &root, &dep2, &log);
        let mut d = doc;
        d["finished_at"] = json!(util::now());
        d["seconds"] = json!(t0.elapsed().as_secs_f64());
        let (event, text) = match &outcome {
            Ok((commit, message)) => {
                d["state"] = json!("success");
                d["commit_after"] = json!(commit);
                d["message"] = json!(message);
                ("deploy.finished", format!("{} deployed to {} ({}s){}", l.site, l.server, t0.elapsed().as_secs(), message.as_ref().map(|m| format!(": {m}")).unwrap_or_default()))
            }
            Err((step, why)) => {
                d["state"] = json!("failed");
                d["failed_line"] = json!(step);
                append(&log, &format!("FAILED at {step}: {why}"));
                ("deploy.failed", format!("Deploy of {} to {} failed at {step}", l.site, l.server))
            }
        };
        let _ = ctx.store.ops_put(crate::deploy::DEPLOY, &dep2, &l.site, &d);
        crate::notify::send(&ctx, event, &l.site, &text, json!({ "deployment": dep2, "server": l.server }));
    });
    crate::deploy::get(ctx, &dep)
}

/// The steps of one deploy. An error names the step that failed.
fn run_deploy(ctx: &Ctx, l: &Link, srv: &Server, site: &devd_core::state::Site, root: &Path, dep: &str, log: &Path) -> std::result::Result<(String, Option<String>), (String, String)> {
    let fail = |step: &str, why: String| (step.to_string(), why);
    let vars = crate::deploy::vars(ctx, &l.site).map_err(|e| fail("variables", e.to_string()))?;
    let envs: Vec<(String, String)> = vars.clone().into_iter().collect();
    let commit = crate::deploy::head(ctx, root).unwrap_or_else(|| "unknown".to_string());
    let message = crate::deploy::git(ctx, root, &["log", "-1", "--pretty=%s"]).ok();

    if l.mode == "git" {
        let (script_text, _) = crate::deploy::script(ctx, &l.site).map_err(|e| fail("script", e.to_string()))?;
        let script_text = crate::deploy::substitute(&script_text, &vars);
        let lines: Vec<String> = crate::deploy::lines(&script_text, Path::new(&l.path), &l.branch, false).into_iter().map(|(line, _)| line).collect();
        append(log, &format!("running {} line(s) on {}", lines.len(), srv.name));
        let r = servers::run(ctx, srv, &format!("bash -c {}", shq(&git_script(l, &lines))), None, Duration::from_secs(3600), false).map_err(|e| fail("ssh", e.to_string()))?;
        append(log, &format!("{}{}", r.stdout, r.stderr));
        if r.code != 0 {
            return Err(fail("the deploy script on the server", format!("exit {}", r.code)));
        }
        return Ok((commit, message));
    }

    for cmd in &l.build {
        append(log, &format!("(build) $ {cmd}"));
        if crate::run_line_with(ctx, site, root, cmd, log, &envs).exit != 0 {
            return Err(fail(cmd, "the local build failed".to_string()));
        }
    }
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let files = file_list(root, &l.include).map_err(|e| fail("packaging", e.to_string()))?;
    append(log, &format!("packaging {} file(s)", files.len()));
    let archive = package(ctx, root, &files, dep).map_err(|e| fail("packaging", e.to_string()))?;
    append(log, &format!("uploading {:.1} MB to {}", std::fs::metadata(&archive).map(|m| m.len()).unwrap_or(0) as f64 / 1_048_576.0, srv.name));
    let up = servers::run(ctx, srv, &format!("mkdir -p {p}/releases {p}/shared && cat > {p}/releases/{ts}.tgz", p = shq(&l.path)), Some(&archive), Duration::from_secs(3600), false);
    let _ = std::fs::remove_file(&archive);
    let up = up.map_err(|e| fail("upload", e.to_string()))?;
    if up.code != 0 {
        return Err(fail("upload", format!("exit {}: {}", up.code, up.stderr.trim())));
    }
    let steps: Vec<String> = if l.steps.is_empty() { default_steps(root) } else { l.steps.clone() };
    let script = release_script(l, &ts, &steps, &commit);
    append(log, &format!("switching to release {ts} after {} step(s)", steps.len()));
    let r = servers::run(ctx, srv, &format!("bash -c {}", shq(&script)), None, Duration::from_secs(3600), false).map_err(|e| fail("remote steps", e.to_string()))?;
    append(log, &format!("{}{}", r.stdout, r.stderr));
    if r.code != 0 {
        let last = format!("{}{}", r.stdout, r.stderr).lines().rev().find(|x| x.starts_with("$ ")).map(|x| x.trim_start_matches("$ ").to_string()).unwrap_or_else(|| "the steps on the server".to_string());
        return Err(fail(&last, format!("exit {}; the running site was not changed", r.code)));
    }
    Ok((commit, message))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link() -> Link {
        Link { id: "shop@hetzner".into(), site: "shop".into(), server: "hetzner".into(), path: "/var/www/shop".into(), mode: "releases".into(), branch: "main".into(), web_dir: "public".into(), php: "8.4".into(), keep: 5, domain: Some("shop.example.com".into()), build: vec![], include: vec![], steps: vec![] }
    }

    #[test]
    fn a_remote_path_is_a_safe_place_to_own() {
        for ok in ["/var/www/shop", "/home/ploi/shop.com", "/srv/apps/a-b_c"] {
            assert!(valid_remote_path(ok), "{ok}");
        }
        for bad in ["var/www/shop", "/", "/var", "/etc/nginx", "/usr/local/x", "/var/www/..", "/var//www", "/var/www/shop/", "/var/www/a b", "/var/www/a;b", "/root/x", "/var/www/$HOME"] {
            assert!(!valid_remote_path(bad), "{bad}");
        }
    }

    #[test]
    fn a_release_script_switches_atomically_only_after_the_steps_and_never_ships_secrets() {
        let s = release_script(&link(), "20261023-153000", &["composer install --no-dev".to_string(), "php artisan migrate --force".to_string()], "abc123");
        let at = |needle: &str| s.find(needle).unwrap_or_else(|| panic!("{needle} in {s}"));
        assert!(s.starts_with("set -e\n"), "a failure stops everything");
        assert!(at("tar -xzf") < at("composer install") && at("composer install") < at("migrate --force") && at("migrate --force") < at("mv -Tf current.new current"), "{s}");
        assert!(at("shared/.env is missing") < at("tar -xzf") || at("shared/.env is missing") < at("composer install"));
        assert!(s.contains("ln -sf \"$(pwd)/shared/.env\" \"$R/.env\"") && s.contains("ln -s \"$(pwd)/shared/storage\" \"$R/storage\""));
        assert!(s.contains("tail -n +6"), "keep 5 means drop from the sixth: {s}");
        assert!(s.contains("sudo -n service php8.4-fpm reload"), "{s}");
        assert!(s.contains("'/var/www/shop'") || s.contains("cd '/var/www/shop'"), "{s}");
    }

    #[test]
    fn a_git_script_pulls_then_runs_the_deploy_script_as_written() {
        let s = git_script(&link(), &["composer install".to_string(), "echo \"\" | sudo -S service php8.4-fpm reload".to_string()]);
        assert!(s.contains("git pull --ff-only origin 'main'") && s.contains("echo \"\" | sudo -S service php8.4-fpm reload"), "{s}");
        assert!(s.find("git pull").unwrap() < s.find("composer install").unwrap());
    }

    #[test]
    fn a_rollback_names_a_release_or_goes_to_the_one_before() {
        assert!(rollback_script(&link(), None).contains("grep -B1 -x"));
        let named = rollback_script(&link(), Some("20261022-090000"));
        assert!(named.contains("target='20261022-090000'") && named.contains("mv -Tf current.new current"), "{named}");
    }

    #[test]
    fn the_nginx_block_serves_the_current_release_through_php_fpm_and_hides_dotfiles() {
        let c = nginx_vhost(&link()).unwrap();
        assert!(c.contains("server_name shop.example.com;") && c.contains("root /var/www/shop/current/public;") && c.contains("php8.4-fpm.sock"), "{c}");
        assert!(c.contains("location ~ /\\.(?!well-known)"), "{c}");
        let mut git = link();
        git.mode = "git".into();
        assert!(nginx_vhost(&git).unwrap().contains("root /var/www/shop/public;"));
        let mut none = link();
        none.domain = None;
        assert!(nginx_vhost(&none).is_err());
    }

    #[test]
    fn the_provision_script_is_checked_idempotent_and_honest_about_what_it_installs() {
        let s = provision_script(&Stack::default()).unwrap();
        assert!(s.starts_with("set -eu\n") && s.contains("must run as root") && s.contains("ubuntu|debian"));
        assert!(s.contains("php8.4-fpm") && s.contains("mariadb-server") && s.contains("certbot") && s.contains("ufw allow OpenSSH") && s.contains("devd-provisioned"));
        assert!(s.contains("sha384") && s.contains("composer.github.io/installer.sig"), "Composer is checked against its published signature");
        let mut lean = Stack { php: "8.3".into(), database: None, redis: true, node: false, supervisor: false };
        let t = provision_script(&lean).unwrap();
        assert!(t.contains("php8.3-fpm") && t.contains("redis-server") && !t.contains("mariadb") && !t.contains("nodesource") && !t.contains("supervisor"));
        lean.php = "8;rm".into();
        assert!(provision_script(&lean).is_err());
        assert!(provision_script(&Stack { database: Some("oracle".into()), ..Stack::default() }).is_err());
    }

    #[test]
    fn what_ships_leaves_out_secrets_dependencies_and_logs_and_includes_the_build() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        for f in ["artisan", "app/Models/User.php", ".env", ".env.production", "vendor/autoload.php", "node_modules/x/index.js", "storage/logs/laravel.log", "public/build/app.js", "public/index.php", "database/database.sqlite"] {
            let p = r.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "x").unwrap();
        }
        let files = file_list(r, &["public/build".to_string()]).unwrap();
        assert!(files.contains(&"artisan".to_string()) && files.contains(&"app/Models/User.php".to_string()) && files.contains(&"public/build/app.js".to_string()) && files.contains(&"public/index.php".to_string()));
        for banned in [".env", ".env.production", "vendor/autoload.php", "node_modules/x/index.js", "storage/logs/laravel.log", "database/database.sqlite"] {
            assert!(!files.contains(&banned.to_string()), "{banned} must not ship: {files:?}");
        }
    }

    #[test]
    fn default_steps_and_build_follow_the_project() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("composer.json"), "{}").unwrap();
        std::fs::write(d.path().join("artisan"), "").unwrap();
        std::fs::write(d.path().join("package.json"), "{\"scripts\":{\"build\":\"vite build\"}}").unwrap();
        std::fs::write(d.path().join("package-lock.json"), "{}").unwrap();
        let steps = default_steps(d.path());
        assert!(steps[0].starts_with("composer install --no-dev") && steps.iter().any(|s| s == "php artisan migrate --force"));
        let (build, include) = default_build(d.path());
        assert_eq!(build, vec!["npm ci", "npm run build"]);
        assert_eq!(include, vec!["public/build"]);
    }

    #[test]
    fn php_versions_are_checked() {
        assert!(valid_php("8.4") && valid_php("8.10") && !valid_php("8") && !valid_php("8.4.1") && !valid_php("x.y") && !valid_php("8.4;ls"));
    }
}
