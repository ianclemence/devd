/* devd application logic.
 *
 * The whole file is written against one rule: **the UI has no privileges**.
 * Every button calls a capability through `POST /api/invoke`, which means every
 * button is subject to the same tier check, the same approval gate and the same
 * audit row as the equivalent command in a terminal. Where the UI appears to
 * decide something - which button is dangerous, what to confirm first - it is
 * deciding how to *describe* an operation, never whether to allow it. The server
 * would refuse an unapproved destructive call even if this file were replaced
 * with something hostile, which is the only reason it is safe for this file to
 * be as straightforward as it is.
 */
(function () {
  "use strict";

  var TOKEN = document.querySelector('meta[name="devd-token"]').content;

  // ---------------------------------------------------------------- transport

  function request(path, init) {
    init = init || {};
    var headers = { Authorization: "Bearer " + TOKEN };
    if (init.body !== undefined) headers["Content-Type"] = "application/json";
    return fetch(path, {
      method: init.method || "GET",
      headers: headers,
      body: init.body
    }).catch(function (e) {
      // No response at all: the server is gone, not refusing. Say so once, plainly.
      connection(false);
      throw new Error("Can't reach devd right now. Nothing was sent or changed.");
    }).then(function (res) {
      // A 401 here means devd restarted and issued a new session: this window's is gone.
      if (res.status === 401) sessionEnded();
      else connection(true);
      return res.text().then(function (text) {
        var body = null;
        try {
          body = text ? JSON.parse(text) : null;
        } catch (e) {
          body = { raw: text };
        }
        return { ok: res.ok, status: res.status, body: body };
      });
    });
  }

  /* While the server does not answer, everything on screen is the last thing it
     said. The banner says that, and the page is marked so it does not read as live. */
  var OFFLINE = false;
  function ensureBanner() {
    var bar = document.getElementById("offline-banner");
    if (!bar) {
      bar = document.createElement("div");
      bar.id = "offline-banner";
      bar.className = "offline-banner";
      bar.setAttribute("role", "alert");
      bar.hidden = true;
      document.body.appendChild(bar);
    }
    return bar;
  }
  function connection(up, quiet) {
    if (up === !OFFLINE) { if (!document.getElementById("offline-banner")) ensureBanner(); return; }
    OFFLINE = !up;
    document.documentElement.classList.toggle("is-offline", OFFLINE);
    var bar = ensureBanner();
    bar.hidden = !OFFLINE;
    bar.innerHTML = OFFLINE
      ? "<b>Can't reach devd right now.</b> What you see is the last thing it told us, so it may be out of date. Nothing you do here is sent until it's back, and this page will reconnect on its own. " +
        '<button type="button" class="btn btn-mini" data-action="reconnect">Try now</button>'
      : "";
    if (el("health-chip")) healthChip();
    if (!OFFLINE && !quiet) toast("ok", "Back in touch with devd");
  }

  function sessionEnded() {
    connection(true, true);
    var bar = document.getElementById("offline-banner");
    if (!bar || bar.dataset.ended) return;
    bar.dataset.ended = "1";
    bar.hidden = false;
    document.documentElement.classList.add("is-offline");
    bar.innerHTML = "<b>devd restarted.</b> This window's session ended with it, so nothing here will go through until you reload. Nothing was changed. " +
      '<button type="button" class="btn btn-mini btn-primary" data-action="reload-page">Reload</button>';
  }

  function errorOf(body, fallback) {
    if (body && body.error && body.error.message) {
      // The message carries the error's code as a prefix ("not found: ...").
      // The code is for machines; the sentence is for people.
      var message = String(body.error.message).replace(/^[a-z][a-z ]*:\s*/, "");
      return message || fallback || "the request failed";
    }
    return fallback || "the request failed";
  }

  /* Dispatch a read-only capability and return its structured data. */
  function read(id, args) {
    return request("/api/invoke", {
      method: "POST",
      body: JSON.stringify({ id: id, args: args || {}, dry_run: false })
    }).then(function (res) {
      if (!res.ok) throw new Error(errorOf(res.body, id + " failed"));
      return res.body.outcome ? res.body.outcome.data : null;
    });
  }

  /* Ask for a plan without applying anything. */
  function plan(id, args) {
    return request("/api/invoke", {
      method: "POST",
      body: JSON.stringify({ id: id, args: args || {}, dry_run: true })
    }).then(function (res) {
      if (!res.ok) throw new Error(errorOf(res.body, id + " could not be planned"));
      return res.body.plan;
    });
  }

  /*
   * Run a capability, handling the approval negotiation.
   *
   * Returns a promise for the outcome, or null when the operation was not
   * applied - because the user cancelled, because the plan could not be made,
   * or because the call failed. Callers treat null as "nothing happened", which
   * is why a cancellation is not an error.
   */
  function run(id, args, opts) {
    opts = opts || {};
    var payload = { id: id, args: args || {}, dry_run: false };
    return request("/api/invoke", {
      method: "POST",
      body: JSON.stringify(payload)
    })
      .then(function (res) {
        if (res.ok && res.body.approval_required) {
          return askApproval(res.body.args === undefined ? Object.assign({}, res.body, { args: args || {} }) : res.body).then(function (granted) {
            if (!granted) {
              return request("/api/deny", {
                method: "POST",
                body: JSON.stringify({ token: res.body.token })
              }).then(function () {
                toast("warn", id + " was not approved");
                return null;
              });
            }
            return request("/api/approve", {
              method: "POST",
              body: JSON.stringify({ token: res.body.token })
            }).then(function (done) {
              if (!done.ok) {
                toast("error", errorOf(done.body, "the approved operation failed"));
                return null;
              }
              var outcome = done.body.outcome;
              if (!opts.quiet) toast("ok", (outcome && outcome.message) || id + " applied");
              return outcome || { ok: true, message: id + " applied" };
            });
          });
        }
        if (!res.ok) {
          toast("error", errorOf(res.body, id + " failed"));
          return null;
        }
        var outcome = res.body.outcome;
        if (!opts.quiet) toast("ok", (outcome && outcome.message) || id + " done");
        return outcome || { ok: true, message: id + " done" };
      })
      .catch(function (e) {
        toast("error", e.message);
        return null;
      });
  }

  /* Run a capability and refresh the view when it changed something. */
  function act(id, args) {
    return run(id, args).then(function (outcome) {
      if (outcome) return refresh().then(function () { return outcome; });
      return null;
    });
  }

  // -------------------------------------------------------------------- state

  var state = {
    view: "home",
    session: null,
    doctor: null,
    health: null,
    projects: [],
    projectCount: 0,
    projectsLoaded: false,
    projectsError: "",
    rosterProject: "",
    rosterCharter: null,
    rosterContext: null,
    projectQuery: "",
    projectSort: "name",
    project: null,
    tab: "changes",
    tabs: {},
    expanded: {},
    diff: null,
    caps: { total: 0, items: [], q: "", group: "" },
    filters: { services: "", sites: "", toolchain: "", runtimes: "", skills: "" },
    filterTimer: null,
    selectedCap: null,
    capArgs: {},
    audit: [],
    palette: { open: false, items: [], index: 0 },
    busy: false,
    server: {
      services: [],
      sites: [],
      runtimes: [],
      certs: [],
      hosts: null,
      ca: null,
      trust: null,
      loaded: false
    },
    siteFormOpen: false,
    certFormOpen: false,

    processes: {
      project: null,
      plan: null,
      status: null,
      error: null,
      busy: false,
      selected: null,
      log: null,
      logQuery: "",
      timer: null,
      loading: false
    },
    shared: {
      project: null,
      tab: "todos",
      notes: [],
      todos: [],
      activeNote: null,
      noteContent: "",
      noteRevision: 0,
      todoOpen: false,
      noteOpen: false,
      busy: false,
      loading: false
    },
    preview: { site: null }
  };

  // ------------------------------------------------------------------ helpers

  var ESCAPES = { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" };

  function esc(value) {
    if (value === null || value === undefined) return "";
    return String(value).replace(/[&<>"']/g, function (c) { return ESCAPES[c]; });
  }

  function attr(value) {
    return esc(value);
  }

  /* Long tool output is bounded once, at render time, with a note that says so.
     The complete text stays in the transcript; the page never tries to lay out
     a megabyte of monospace. */
  function clip(text, max) {
    var s = String(text == null ? "" : text);
    if (s.length <= max) return s;
    return s.slice(0, max) + "\n… " + (s.length - max) + " more characters (full output is in the transcript)";
  }

  function el(id) {
    return document.getElementById(id);
  }

  /*
   * Paging.
   *
   * A list that grows without bound is a list nobody can read. The page size
   * is chosen so that a page fits on a screen, the control says exactly what
   * is showing and what is not, and the count is real.
   */
  var PAGE_SIZE = 50;

  function pager(total, page, action) {
    var pages = Math.max(1, Math.ceil(total / PAGE_SIZE));
    if (pages <= 1) return "";
    var from = page * PAGE_SIZE + 1;
    var to = Math.min(total, (page + 1) * PAGE_SIZE);
    return (
      '<div class="pager">' +
      '<span class="pager-count">Showing ' + from + "–" + to + " of " + total + "</span>" +
      '<div class="pager-buttons">' +
      '<button type="button" class="btn btn-mini" data-action="' + action + '" data-page="' + (page - 1) + '"' + (page === 0 ? " disabled" : "") + ">Previous</button>" +
      '<span class="pager-page">Page ' + (page + 1) + " of " + pages + "</span>" +
      '<button type="button" class="btn btn-mini" data-action="' + action + '" data-page="' + (page + 1) + '"' + (page >= pages - 1 ? " disabled" : "") + ">Next</button>" +
      "</div></div>"
    );
  }

  /*
   * Brand marks.
   *
   * Each one is the owner's own artwork, served from the binary. The map is
   * the list of things devd actually supports: a mark is never shown for
   * something devd cannot do, and a supported thing never goes unmarked.
   */
  var BRANDS = {
    // Model providers.
    anthropic: "anthropic", openai: "openai", deepseek: "deepseek",
    moonshot: "moonshot", zhipu: "zhipu", gemini: "gemini", openrouter: "openrouter",
    ollama: "ollama", "lm-studio": "lmstudio", mistral: "mistral", groq: "groq",
    together: "together", fireworks: "fireworks", xai: "xai", claude: "claude",
    // Services.
    mysql: "mysql", mariadb: "mariadb", postgresql: "postgresql", mongodb: "mongodb",
    redis: "redis", valkey: "valkey", typesense: "typesense", sqlite: "sqlite",
    nginx: "nginx", php: "php", node: "node", composer: "composer", laravel: "laravel",
    xdebug: "xdebug", mailpit: "mailpit", meilisearch: "meilisearch", minio: "minio",
    elasticsearch: "elasticsearch", prometheus: "prometheus", grafana: "grafana",
    docker: "docker", git: "git", sentry: "sentry", opencode: "opencode",
    phpmyadmin: "phpmyadmin", phpstorm: "phpstorm",
    cursor: "cursor", insomnia: "insomnia", postman: "postman", figma: "figma",
    discord: "discord", notion: "notion", obsidian: "obsidian", raycast: "raycast",
    // The toolchain: languages, package managers, version control, mobile.
    npm: "npm", pnpm: "pnpm", yarn: "yarn", bun: "bun", python: "python",
    uv: "uv", poetry: "poetry", ruby: "ruby", bundler: "bundler", rails: "rails",
    go: "go", java: "java", maven: "maven", gradle: "gradle", dotnet: "dotnet",
    rust: "rust", gh: "gh", adb: "android", "android-sdk": "android", expo: "expo",
    "eas-cli": "eascli", "build-tools": "build-tools",
    // The stacks a project can be created from.
    astro: "astro", django: "django", express: "express", fastapi: "fastapi",
    flask: "flask", nextjs: "nextjs", "react-native": "react-native",
    "react-vite": "react-vite", "spring-boot": "spring-boot", sveltekit: "sveltekit",
    slim: "slim", vue: "vue", tailwind: "tailwind", typescript: "typescript",
    javascript: "javascript",
    // Editors and CI.
    vscode: "vscode", sublime: "sublime", xcode: "xcode", jetbrains: "jetbrains",
    intellijidea: "intellijidea", androidstudio: "androidstudio", gitlab: "gitlab",
    jenkins: "jenkins", terraform: "terraform", ansible: "ansible"
  };

  /*
   * The mark for a thing, or nothing.
   *
   * An exact name is the normal case. A name with a version or an instance
   * number after it - "postgresql-16", "anthropic-2" - is the same thing. A
   * name that merely *starts* with a brand is a different thing: "phpstan" is
   * not PHP, and marking it as PHP would be a lie.
   */
  function brandFor(name) {
    if (!name) return null;
    var key = String(name).toLowerCase();
    if (BRANDS[key]) return BRANDS[key];
    var best = null;
    Object.keys(BRANDS).forEach(function (k) {
      if (key.indexOf(k) !== 0) return;
      var rest = key.slice(k.length);
      if (!/^[-_. ]?\d/.test(rest)) return;
      if (!best || k.length > best.length) best = k;
    });
    return best ? BRANDS[best] : null;
  }

  /*
   * Some owners publish a wordmark rather than a square mark. Squeezing a
   * wordmark into a square makes it unreadable, so those are given a wider
   * slot of their own - still their own artwork, never stretched.
   */
  var WIDE_BRANDS = { zhipu: true };

  /* The mark as a small image. The name is the accessible label, and the
     image is decorative: the text beside it already says what it is. */
  function brandImg(name, className) {
    var file = brandFor(name);
    if (!file) return "";
    var ext = file.indexOf("together") === 0 || file === "typesense" || file === "valkey" || file === "xdebug" || file === "vscode" ? "png" : "svg";
    var wide = WIDE_BRANDS[file] ? " brand-dot-wide" : "";
    return '<img class="' + (className || "brand-dot") + wide + '" src="/brands/' + file + "." + ext + '" alt="" loading="lazy" width="18" height="18">';
  }

  function relativeTime(seconds) {
    if (!seconds) return "";
    var diff = Math.floor(Date.now() / 1000) - Number(seconds);
    if (diff < 60) return "just now";
    if (diff < 3600) return Math.floor(diff / 60) + "m ago";
    if (diff < 86400) return Math.floor(diff / 3600) + "h ago";
    if (diff < 2592000) return Math.floor(diff / 86400) + "d ago";
    if (diff < 31536000) return Math.floor(diff / 2592000) + "mo ago";
    return Math.floor(diff / 31536000) + "y ago";
  }

  function shortPath(p, root) {
    if (!p) return "";
    if (root && p.indexOf(root) === 0) {
      var rest = p.slice(root.length).replace(/^[\\/]+/, "");
      return rest || ".";
    }
    return p;
  }

  function dirName(p) {
    var parts = String(p).split(/[\\/]/);
    return parts[parts.length - 1] || p;
  }

  /* Make sure a project picked outside the Assistant is one the composer can
     show. The Home row comes from the same list, but a stale or filtered list
     must not leave the select with no matching option. */
  function rememberProject(path) {
    if (!path) return;
    if (!(state.projects || []).some(function (p) { return p.path === path; })) {
      state.projects = (state.projects || []).concat([{ name: dirName(path), path: path }]);
    }
  }

  /* The series of a PHP version: "8.5.11" is the series "8.5". */
  function devSeries(version) {
    var parts = String(version || "").split(".");
    if (parts.length < 2 || !parts[0] || !parts[1]) return String(version || "");
    return parts[0] + "." + parts[1];
  }

  /*
   * A one-letter badge for a file's state.
   *
   * The letters come from Git's porcelain status, and the order matters: a
   * file that is both modified and deleted is a deletion, because that is
   * what will happen to it. Untracked and conflicted are different states and
   * must not share a badge, or a conflict reads as a harmless new file.
   */
  function statusTag(code) {
    var c = String(code || "").toUpperCase();
    var cls = "tag";
    var letter = c || "?";
    if (c.indexOf("D") >= 0) { cls += " tag-del"; letter = "D"; }
    else if (c.indexOf("C") >= 0) { cls += " tag-add"; letter = "C"; }
    else if (c.indexOf("U") >= 0) { cls += " tag-conf"; letter = "U"; }
    else if (c.indexOf("A") >= 0) { cls += " tag-add"; letter = "A"; }
    else if (c.indexOf("R") >= 0) { cls += " tag-mod"; letter = "R"; }
    else if (c === "?") { cls += " tag-unt"; letter = "U"; }
    else { cls += " tag-mod"; letter = c.charAt(0) || "M"; }
    return '<span class="' + cls + '" title="' + attr(c) + '">' + esc(letter) + "</span>";
  }

  function tierClass(tier) {
    if (tier === "destructive" || tier === "admin") return "pill pill-bad";
    if (tier === "remote") return "pill pill-warn";
    if (tier === "exec") return "pill pill-accent";
    if (tier === "write") return "pill pill-accent";
    return "pill";
  }

  /* A path row action button. */
  function mini(label, action, data, extra) {
    var attrs = "";
    Object.keys(data || {}).forEach(function (k) {
      attrs += " data-" + k + '="' + attr(data[k]) + '"';
    });
    var cls = extra && extra.danger ? "btn btn-mini btn-danger" : "btn btn-mini";
    return (
      '<button type="button" class="' + cls + '" data-action="' + attr(action) + '"' + attrs + ">" +
      esc(label) +
      "</button>"
    );
  }

  function renderPatch(patch, limit) {
    if (!patch) return '<div class="empty">Nothing to show</div>';
    var lines = String(patch).split("\n");
    var cap = limit || 4000;
    var truncated = lines.length > cap;
    if (truncated) lines = lines.slice(0, cap);
    var html = lines
      .map(function (line) {
        var cls = "diff-line d-ctx";
        if (line.indexOf("diff --git") === 0 || line.indexOf("index ") === 0 ||
            line.indexOf("--- ") === 0 || line.indexOf("+++ ") === 0 ||
            line.indexOf("new file") === 0 || line.indexOf("deleted file") === 0 ||
            line.indexOf("similarity index") === 0 || line.indexOf("rename ") === 0) {
          cls = "diff-line d-meta";
        } else if (line.indexOf("@@") === 0) {
          cls = "diff-line d-hunk";
        } else if (line.charAt(0) === "+") {
          cls = "diff-line d-add";
        } else if (line.charAt(0) === "-") {
          cls = "diff-line d-del";
        }
        return '<div class="' + cls + '">' + (esc(line) || "&nbsp;") + "</div>";
      })
      .join("");
    if (truncated) {
      html += '<div class="diff-line d-meta">... truncated after ' + cap + " lines</div>";
    }
    return '<div class="diff">' + html + "</div>";
  }

  // ------------------------------------------------------------------- toasts

  /* Tell a screen reader what the eye gets from a toast. The toast itself is hidden from
     assistive technology so nothing is read twice; two live regions that exist before
     anything is put in them carry the words: polite for news, assertive for a failure. */
  function announce(kind, text) {
    var region = el(kind === "error" ? "live-assertive" : "live-polite");
    if (!region) return;
    region.textContent = "";
    // A change after a tick is announced even when the same words come twice in a row.
    setTimeout(function () { region.textContent = text; }, 30);
  }

  // The kind is said in a word as well as the edge colour.
  var TOAST_WORD = { error: "Didn't work", warn: "Heads up", ok: "Done" };

  function toast(kind, message, detail) {
    announce(kind, message + (detail ? ". " + detail : ""));
    var node = document.createElement("div");
    node.className = "toast toast-" + kind;
    node.setAttribute("aria-hidden", "true");
    node.innerHTML =
      '<div class="toast-msg">' + (TOAST_WORD[kind] ? '<b class="toast-kind">' + TOAST_WORD[kind] + "</b>" : "") + esc(message) +
      (detail ? '<div class="toast-detail">' + esc(detail) + "</div>" : "") +
      '</div><button type="button" data-action="dismiss-toast" aria-label="Dismiss">x</button>';
    el("toasts").appendChild(node);
    // The entrance is a transition, so it can be interrupted and retargeted.
    requestAnimationFrame(function () { node.classList.add("toast-in"); });
    node.dataset.timer = String(setTimeout(function () { dismissToast(node); }, kind === "error" ? 9000 : 4200));
  }

  function dismissToast(node) {
    if (!node || node.dataset.leaving) return;
    node.dataset.leaving = "1";
    if (node.dataset.timer) clearTimeout(Number(node.dataset.timer));
    node.classList.remove("toast-in");
    node.classList.add("toast-out");
    setTimeout(function () { if (node.parentNode) node.parentNode.removeChild(node); }, 200);
  }

  function copyText(text, done) {
    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(text).then(done, function () { legacyCopy(text, done); });
    } else {
      legacyCopy(text, done);
    }
  }

  function legacyCopy(text, done) {
    var area = document.createElement("textarea");
    area.value = text;
    area.setAttribute("readonly", "");
    area.style.cssText = "position:fixed;top:0;left:0;opacity:0";
    document.body.appendChild(area);
    area.select();
    try {
      document.execCommand("copy");
      done();
    } catch (e) {
      toast("error", "The browser refused clipboard access");
    }
    document.body.removeChild(area);
  }

  // ------------------------------------------------------------------- drawer

  /* Where focus was when the drawer opened, so closing it returns the person to the
     control they used. Opening a drawer over another one keeps the first origin. */
  var drawerOrigin = null;

  function openDrawer(title, html) {
    var wasOpen = !el("drawer").hidden;
    if (!wasOpen) drawerOrigin = document.activeElement;
    el("drawer-title").textContent = title;
    el("drawer-body").innerHTML = html;
    el("drawer").hidden = false;
    el("drawer-scrim").hidden = false;
    // Focus moves into the drawer: the close button is the first thing a keyboard
    // user can act on, and a screen reader announces the dialog's name from it.
    var close = document.querySelector("#drawer .drawer-close");
    if (close && !wasOpen) close.focus();
  }

  function closeDrawer() {
    var wasOpen = !el("drawer").hidden;
    el("drawer").hidden = true;
    el("drawer-scrim").hidden = true;
    if (wasOpen && drawerOrigin && document.body.contains(drawerOrigin) && typeof drawerOrigin.focus === "function") {
      drawerOrigin.focus();
    }
    drawerOrigin = null;
  }

  // ------------------------------------------------------------------- dialog
  //
  // A question the person answers in the application's own style, in place of the browser's
  // alert boxes: a title, a sentence or two, optionally one text field, and the buttons in the
  // same place as every other dialog (Cancel, then the action, at the right).

  function uiDialog(o) {
    return new Promise(function (resolve) {
      var d = el("dialog"), ok = el("dialog-ok"), cancel = el("dialog-cancel"), inp = el("dialog-input");
      var back = document.activeElement;
      el("dialog-title").textContent = o.title || "";
      el("dialog-text").textContent = o.text || "";
      el("dialog-text").hidden = !o.text;
      var ic = el("dialog-icon");
      ic.className = "modal-icon" + (o.danger ? " modal-icon-danger" : "");
      ic.firstChild.firstChild.setAttribute("href", "#" + (o.danger ? "i-warn" : o.input ? "i-pencil" : "i-info"));
      inp.hidden = !o.input;
      if (o.input) { inp.value = o.input.value || ""; inp.placeholder = o.input.placeholder || ""; }
      ok.textContent = o.confirm || "OK";
      ok.className = "btn " + (o.danger ? "btn-danger" : "btn-go");
      var close = function (res) {
        d.hidden = true;
        ok.onclick = cancel.onclick = d.onclick = d.onkeydown = null;
        el("dialog-close").onclick = null;
        if (back && back.focus) back.focus();
        resolve(res);
      };
      ok.onclick = function () { close({ ok: true, value: inp.value.trim() }); };
      cancel.onclick = function () { close({ ok: false }); };
      el("dialog-close").onclick = function () { close({ ok: false }); };
      d.onclick = function (e) { if (e.target === d) close({ ok: false }); };
      d.onkeydown = function (e) {
        if (e.key === "Escape") { e.stopPropagation(); close({ ok: false }); }
        else if (e.key === "Enter" && e.target.tagName !== "BUTTON") { e.preventDefault(); ok.click(); }
      };
      d.hidden = false;
      var first = o.input ? inp : (o.danger ? cancel : ok);
      first.focus();
      if (o.input) inp.select();
    });
  }

  function uiConfirm(o) { return uiDialog(o).then(function (r) { return r.ok; }); }
  function uiPrompt(o) { return uiDialog(o).then(function (r) { return r.ok ? r.value : null; }); }

  // ------------------------------------------------------------------- picker
  //
  // Choosing a folder is a thing a person does with a file browser, not with a
  // text box. This dialog shows the device the way the operating system does:
  // the places work is kept, the path you are in, and the folders that are
  // there. The desktop shell can also put its own native dialog in front of
  // this one, and when the page is running in a plain browser that button is
  // simply not offered, because there is no native dialog to offer.
  //
  // The path that comes back is only ever a string here. Everything that acts
  // on it - detecting the project, registering it - goes through a capability
  // that inspects it first.

  var picker = {
    open: false,
    title: "Choose a folder",
    note: "Move through your device and pick the folder that holds the project.",
    confirm: "Use this folder",
    /** What the caller wants to do with the folder. */
    intent: "directory",
    /** Where the browser should open. */
    path: null,
    /** The listing currently shown. */
    listing: null,
    /** The folder the person has highlighted, and that the confirm button takes. */
    selected: null,
    /** What the detection found in the highlighted folder. */
    inspect: null,
    /** Whether the native dialog is available in this surface. */
    native: false,
    /** Resolves the promise handed back to the caller. */
    resolve: null,
    /** The element the picker was opened from, so focus can go back to it. */
    opener: null
  };

  /* Open the picker and resolve with the chosen folder, or null. */
  function openPicker(options) {
    options = options || {};
    picker.title = options.title || "Choose a folder";
    picker.note = options.note || "Move through your device and pick the folder that holds the project.";
    picker.confirm = options.confirm || "Use this folder";
    picker.intent = options.intent || "directory";
    picker.listing = null;
    picker.inspect = null;
    picker.selected = options.start || null;
    picker.opener = document.activeElement;
    picker.native = false;

    el("picker-title").textContent = picker.title;
    el("picker-note").textContent = picker.note;
    el("picker-choose").textContent = picker.confirm;
    el("picker-path-input").value = picker.selected || "";
    el("picker").hidden = false;
    picker.open = true;
    // The desktop shell can put the operating system's own dialog in front of
    // this one; a plain browser cannot, and the button is not offered there.
    picker.native = !!(state.session && state.session.native_picker);

    var start = picker.selected;
    if (!start) {
      read("fs.browse", {}).then(function (places) {
        picker.places = places || [];
        paintPicker();
      }).catch(function (e) {
        toast("error", e.message);
      });
    }
    return new Promise(function (resolve) {
      picker.resolve = resolve;
    }).then(function (path) {
      picker.open = false;
      el("picker").hidden = true;
      if (picker.opener && picker.opener.focus) {
        try { picker.opener.focus(); } catch (e) { /* the opener is gone */ }
      }
      picker.resolve = null;
      return path;
    });
  }

  function closePicker(path) {
    if (!picker.resolve) return;
    var done = picker.resolve;
    picker.resolve = null;
    done(path || null);
  }

  /* Load a directory and show it. */
  function pickerGo(path) {
    var current = picker.listing ? picker.listing.path : null;
    return read("fs.browse", { path: path }).then(function (listing) {
      picker.listing = listing;
      picker.selected = listing.path;
      picker.inspect = null;
      el("picker-path-input").value = listing.path;
      if (current !== listing.path) {
        // The person moved: what used to be highlighted is no longer relevant.
        picker.inspect = null;
      }
      return pickerInspect(listing.path);
    }).then(function () {
      paintPicker();
    }).catch(function (e) {
      toast("error", e.message);
      paintPicker();
    });
  }

  /* Ask what is in the highlighted folder. */
  function pickerInspect(path) {
    if (picker.intent !== "project") {
      picker.inspect = null;
      return Promise.resolve(null);
    }
    return read("project.detect", { path: path }).then(function (ctx) {
      picker.inspect = ctx;
      return ctx;
    }).catch(function (e) {
      picker.inspect = { error: e.message };
      return null;
    });
  }

  function paintPicker() {
    if (!picker.open) return;
    var listing = picker.listing;
    var native = el("picker-native");
    native.hidden = !picker.native;

    // Places to start from.
    var places = el("picker-places");
    var chips = [];
    (picker.places || []).forEach(function (p) {
      chips.push(
        '<button type="button" class="picker-place" data-action="picker-goto" data-path="' + attr(p.path) + '"' +
        (listing && listing.path === p.path ? ' aria-current="true"' : "") + ">" + esc(p.label) + "</button>"
      );
    });
    if (listing) {
      (listing.roots || []).forEach(function (r) {
        chips.push('<span class="picker-place-sep" aria-hidden="true">·</span>');
        chips.push(
          '<button type="button" class="picker-place" data-action="picker-goto" data-path="' + attr(r.path) + '">' +
          esc(r.label) + "</button>"
        );
      });
    }
    places.innerHTML = chips.join("");

    // The path, as a trail of places you can step back through.
    var crumbs = el("picker-crumbs");
    if (listing) {
      var parts = (listing.crumbs || []).map(function (c, i) {
        var current = i === listing.crumbs.length - 1;
        return (i ? '<span class="picker-crumb-sep" aria-hidden="true">/</span>' : "") +
          '<button type="button" class="picker-crumb" data-action="picker-goto" data-path="' + attr(c.path) + '"' +
          (current ? ' aria-current="true"' : "") + ">" + esc(c.label) + "</button>";
      });
      crumbs.innerHTML = parts.join("");
    } else {
      crumbs.innerHTML = "";
    }
    el("picker-up").disabled = !listing || !listing.parent;

    // The folders.
    var list = el("picker-list");
    if (!listing) {
      list.innerHTML = '<div class="picker-inspect-empty" style="padding:var(--s3)">Choose one of the places above to begin.</div>';
    } else {
      var folders = (listing.entries || []).filter(function (e) { return e.kind === "directory"; });
      var files = (listing.entries || []).filter(function (e) { return e.kind !== "directory"; });
      var rows = "";
      folders.concat(files).forEach(function (e) {
        var isFolder = e.kind === "directory";
        var selected = picker.selected === e.path;
        rows +=
          '<button type="button" class="picker-item' + (isFolder ? "" : " is-file") + '" role="option"' +
          ' aria-selected="' + (selected ? "true" : "false") + '"' +
          ' data-action="' + (isFolder ? "picker-goto" : "picker-pick-file") + '"' +
          ' data-path="' + attr(e.path) + '"' +
          ' title="' + attr(e.path) + '">' +
          '<svg class="icon" aria-hidden="true"><use href="#' + (isFolder ? "i-folder" : "i-code") + '"></use></svg>' +
          '<span class="picker-item-name">' + esc(e.name) + "</span>" +
          (e.symlink ? '<span class="picker-item-kind">link</span>' : "") +
          '<span class="picker-item-kind">' + (isFolder ? "folder" : "file") + "</span>" +
          "</button>";
      });
      if (!rows) {
        rows = '<div class="picker-inspect-empty" style="padding:var(--s3)">This folder is empty.</div>';
      }
      if (listing.warning) {
        rows += '<div class="picker-inspect-warn" style="padding:var(--s2) var(--s3)">' + esc(listing.warning) + "</div>";
      }
      if (listing.truncated) {
        rows += '<div class="picker-inspect-empty" style="padding:var(--s2) var(--s3)">Only the first entries are shown. Type a path to go straight to a folder.</div>';
      }
      list.innerHTML = rows;
    }

    paintPickerInspect();
  }

  function paintPickerInspect() {
    var box = el("picker-inspect");
    var ctx = picker.inspect;
    if (!ctx) {
      box.innerHTML = '<div class="picker-inspect-empty">Nothing is selected yet.</div>';
      return;
    }
    if (ctx.error) {
      box.innerHTML = '<div class="picker-inspect-warn">' + esc(ctx.error) + "</div>";
      return;
    }
    var lines = [];
    lines.push(
      '<div class="picker-inspect-title">' + esc(ctx.name || "This folder") + "</div>"
    );
    var stack = ctx.primary || (ctx.stacks && ctx.stacks[0] ? ctx.stacks[0].name : null);
    // "Uncertain" only means something when there is a stack to be unsure
    // about. A folder that simply is not a project is not uncertain.
    var unsure = !!(ctx.ambiguous && stack);
    lines.push(
      '<div class="picker-inspect-line"><span class="k">Detected</span><span class="v">' +
      (stack ? esc(stack) : '<span class="picker-inspect-empty">no known stack</span>') +
      (unsure ? ' <span class="pill pill-warn">uncertain</span>' : "") +
      "</span></div>"
    );
    if (ctx.package_manager) {
      lines.push(
        '<div class="picker-inspect-line"><span class="k">Packages</span><span class="v">' + esc(ctx.package_manager) + "</span></div>"
      );
    }
    if (ctx.runtimes && ctx.runtimes.length) {
      lines.push(
        '<div class="picker-inspect-line"><span class="k">Needs</span><span class="v">' +
        esc(ctx.runtimes.map(function (r) { return r.tool + " " + r.constraint; }).join(", ")) +
        "</span></div>"
      );
    }
    if (ctx.git && ctx.git.repository) {
      lines.push(
        '<div class="picker-inspect-line"><span class="k">Git</span><span class="v">' +
        esc(ctx.git.branch ? "repository, on " + ctx.git.branch : "repository") + "</span></div>"
      );
    }
    if (ctx.missing && ctx.missing.length) {
      lines.push(
        '<div class="picker-inspect-line"><span class="k">Missing</span><span class="v picker-inspect-warn">' +
        esc(ctx.missing.map(function (m) { return m.tool; }).join(", ")) +
        "</span></div>"
      );
    }
    if (ctx.warnings && ctx.warnings.length) {
      lines.push(
        '<div class="picker-inspect-line"><span class="k">Notes</span><span class="v">' +
        esc(ctx.warnings.join(" ")) + "</span></div>"
      );
    }
    box.innerHTML = lines.join("");
  }

  // -------------------------------------------------------------------- modal

  var pendingDecision = null;

  /* Present a plan and resolve to true (approved) or false (declined). */
  function askApproval(view) {
    return new Promise(function (resolve) {
      pendingDecision = resolve;

      var plan = view.plan;
      var destructive = !!(plan && plan.destructive) || view.tier === "destructive" || view.tier === "admin";

      el("modal-kicker").textContent = view.capability;
      el("modal-title").textContent = view.summary || view.capability;
      var tierPill = el("modal-tier");
      tierPill.className = tierClass(view.tier);
      tierPill.textContent = view.tier;

      var parts = [];
      if (destructive) {
        parts.push(
          '<div class="notice notice-bad"><strong>This can destroy work.</strong> ' +
          "Read the steps below before approving.</div>"
        );
      }
      if (view.description) {
        parts.push('<div class="dim">' + esc(view.description) + "</div>");
      }
      if (plan) {
        if (plan.summary) {
          parts.push(
            '<div class="panel"><div class="panel-head">' + esc(plan.summary) + "</div>" +
            (plan.steps && plan.steps.length
              ? '<div class="panel-body"><ul class="steps">' +
                plan.steps.map(function (s) { return "<li>" + esc(s) + "</li>"; }).join("") +
                "</ul></div>"
              : "") +
            "</div>"
          );
        }
        if (plan.effects && plan.effects.length) {
          parts.push(
            '<div class="inline">' +
            plan.effects.map(function (e) { return '<span class="pill">' + esc(e) + "</span>"; }).join("") +
            "</div>"
          );
        }
        if (plan.data && typeof plan.data === "object") {
          parts.push(
            '<div><div class="field-label">Plan detail</div><pre class="patch-block">' +
            esc(JSON.stringify(plan.data, null, 2)) + "</pre></div>"
          );
        }
      } else {
        parts.push(
          '<div class="notice notice-warn">This capability cannot produce a plan, ' +
          "so there is no preview of what it will do.</div>"
        );
      }
      parts.push(
        '<div><div class="field-label">Arguments</div><pre class="patch-block">' +
        esc(JSON.stringify(view.args, null, 2)) + "</pre></div>"
      );

      el("modal-body").innerHTML = parts.join("");
      el("modal").hidden = false;

      var confirm = el("modal-confirm");
      confirm.className = "btn " + (destructive ? "btn-danger" : "btn-go");
      confirm.textContent = destructive ? "I understand - run it" : "Approve and run";
      confirm.focus();

      /* Show the deadline the server set, counting down. When it passes the
         token is already dead, so offering the button would be a lie. */
      var left = Math.max(1, Math.round((view.expires_in_ms || 120000) / 1000));
      var expiry = el("modal-expiry");
      var timer = setInterval(function () {
        left -= 1;
        if (left <= 0) {
          clearInterval(timer);
          expiry.textContent = "expired - request the operation again";
          confirm.disabled = true;
          return;
        }
        expiry.textContent = "expires in " + left + "s";
      }, 1000);
      expiry.textContent = "expires in " + left + "s";

      el("modal").dataset.timer = String(timer);
    });
  }

  function decideModal(granted) {
    var timer = el("modal").dataset.timer;
    if (timer) clearInterval(Number(timer));
    el("modal").hidden = true;
    el("modal-confirm").disabled = false;
    if (pendingDecision) {
      pendingDecision(granted);
      pendingDecision = null;
    }
  }

  // ------------------------------------------------------------------ palette

  // ------------------------------------------------------------- global search
  //
  // One box for what a developer reaches for: places, projects, files, text in
  // a project, conversations, models, services, and things to do. Nothing is
  // pre-indexed; slow sources are queried live and the footer says what was and
  // was not searched. Prefixes narrow it: > do, / files, ~ text, @ projects,
  // # conversations, ? ask the assistant.

  var PAL = { seq: 0, timer: null, sources: [], recents: [] };

  function palRecents() {
    try { return JSON.parse(localStorage.getItem("devd.recents") || "[]"); } catch (e) { return []; }
  }
  function palRemember(item) {
    try {
      var list = palRecents().filter(function (r) { return r.key !== item.key; });
      list.unshift({ key: item.key, title: item.title, detail: item.detail || "", group: item.group });
      localStorage.setItem("devd.recents", JSON.stringify(list.slice(0, 8)));
    } catch (e) { /* storage may be blocked; recents are a convenience */ }
  }

  var GO_TO = [
    ["home", "Home", "what needs you, what is running"], ["inbox", "Needs you", "approvals and questions from agents"],
    ["projects", "Projects", "repositories, folders and stacks"], ["sites", "Sites", "addresses, serving and certificates"],
    ["assistant", "Assistant", "talk to the built-in agent"], ["team", "Agent runs", "goals planned, approved and run by the team"],
    ["agents", "Team", "your team and what each member does"],
    ["create", "New project", "start from a stack"], ["develop", "Stacks and tasks", "detect and run project tasks"],
    ["schedules", "Schedules", "team runs on a clock"], ["shared", "Shared notes", "notes agents share"],
    ["mcp", "MCP servers", "tools from other programs"],
    ["processes", "Processes", "supervised project processes and logs"], ["overview", "Environment", "services, runtimes and sites"],
    ["services", "Services", "nginx, PHP, MariaDB and others"],
    ["toolchain", "Tools", "CLI tools and package managers"], ["runtimes", "Runtimes", "language versions and PHP settings"],
    ["databases", "Databases", "database users and backups"], ["remote", "Remote", "servers, SSH keys and Ploi"],
    ["certificates", "Certificates", "local HTTPS"], ["hosts", "Hosts", "hostname entries"],
    ["providers", "Models", "AI providers, keys and models"], ["activity", "Activity", "what was done, by whom"],
    ["system", "Diagnostics", "health checks and fixes"], ["capabilities", "API and tools", "every capability, for scripts and agents"]
  ];

  function palActions(q) {
    var acts = [
      { key: "a:new", title: "Create a project", detail: stackCount() ? "from " + stackCount() + " stacks" : "from a stack", run: function () { navigate("create"); } },
      { key: "a:import", title: "Import an existing project", detail: "detect its stack and tasks", run: function () { navigate("develop"); } },
      { key: "a:terminal", title: "Open a terminal in this project", detail: "its pinned runtimes first on PATH", run: function () { var p = palContext(); if (!p) return toast("warn", "Choose a project first"); run("project.terminal", { path: p }); } },
      { key: "a:provider", title: "Add an AI provider", detail: "key and default model", run: function () { navigate("providers"); } },
      { key: "a:models", title: "Refresh the official model list", detail: "reads Anthropic's documentation", run: function () { run("models.refresh_official", {}); } },
      { key: "a:health", title: "Check system health", detail: "re-runs every check", run: function () { ACTIONS["health-recheck"](); } },
      { key: "a:up", title: "Start all services", detail: "asks you first", ask: true, run: function () { act("stack.up", {}); } },
      { key: "a:down", title: "Stop all services", detail: "asks you first", ask: true, run: function () { act("stack.down", {}); } },
      { key: "a:newchat", title: "New assistant conversation", detail: "", run: function () { ACTIONS["chat-new"](); navigate("assistant"); } }
    ];
    return acts.map(function (a) { a.group = "Do"; return a; });
  }

  function palLocal(q) {
    var toks = q.toLowerCase().split(/\s+/).filter(Boolean);
    function hit(text, name) {
      var h = text.toLowerCase(), n = name.toLowerCase(), sc = 0;
      for (var i = 0; i < toks.length; i++) {
        var t = toks[i];
        if (n === t) sc += 100; else if (n.indexOf(t) === 0) sc += 80; else if (n.indexOf(t) >= 0) sc += 60; else if (h.indexOf(t) >= 0) sc += 30; else return -1;
      }
      return toks.length ? sc : 0;
    }
    var out = [];
    GO_TO.forEach(function (g) {
      var sc = hit(g[1] + " " + g[2], g[1]);
      if (sc >= 0) out.push({ key: "g:" + g[0], group: "Go to", title: g[1], detail: g[2], score: sc + 5, run: function () { navigate(g[0]); } });
    });
    palActions(q).forEach(function (a) {
      var sc = hit(a.title + " " + a.detail, a.title);
      if (sc >= 0) { a.score = sc; out.push(a); }
    });
    state.server.services.forEach(function (sv) {
      var sc = hit(sv.name, sv.name);
      if (sc >= 0 && toks.length) out.push({ key: "svc:" + sv.name, group: "Services", title: sv.name, detail: sv.running ? "running" : "stopped", score: sc + 10, run: function () { navigate("services"); } });
    });
    state.server.sites.forEach(function (st) {
      var nm = st.name || st.host || "";
      var sc = hit(nm + " " + (st.host || ""), nm);
      if (sc >= 0 && toks.length) out.push({ key: "site:" + nm, group: "Sites", title: nm, detail: st.host || "", score: sc + 10, run: function () { openSite(nm); } });
    });
    return out;
  }

  function palRemote(r) {
    var out = [];
    r.results.forEach(function (x) {
      var base = { key: x.kind + ":" + (x.path || x.id || x.title), title: x.title, detail: x.detail, score: x.score };
      if (x.kind === "project") out.push(Object.assign(base, { group: "Projects", run: function () { DEV.path = x.path; DEV.ctx = null; DEV.detect = null; navigate("develop"); } }));
      else if (x.kind === "file" || x.kind === "text") out.push(Object.assign(base, { group: x.kind === "file" ? "Files" : "Text in project", title: x.kind === "text" ? x.title : x.title, run: function () {
        read("fs.read", { path: x.path, offset: Math.max(1, (x.line || 1) - 5), limit: 80 }).then(function (d) { openDrawer(x.path.split(/[\\/]/).pop(), '<div class="mono dim nowrap" title="' + attr(x.path) + '">' + esc(x.path) + '</div><pre class="log-block">' + esc(d.content || JSON.stringify(d, null, 2)) + "</pre>"); }).catch(function (e) { toast("error", e.message); });
      } }));
      else if (x.kind === "conversation") out.push(Object.assign(base, { group: "Conversations", run: function () { ACTIONS["open-thread"]({ dataset: { id: x.id } }); } }));
      else if (x.kind === "provider") out.push(Object.assign(base, { group: "Models", run: function () { navigate("providers"); } }));
      else if (x.kind === "model") out.push(Object.assign(base, { group: "Models", run: function () { navigate("providers"); } }));
      else if (x.kind === "task") out.push(Object.assign(base, { group: "Tasks", run: function () { navigate("develop"); } }));
      else if (x.kind === "activity") out.push(Object.assign(base, { group: "Activity", run: function () { navigate("activity"); } }));
    });
    return out;
  }

  function palContext() { return DEV.path || (state.project && state.project.path) || (state.projects[0] && state.projects[0].path) || ""; }

  function palParse(raw) {
    var m = /^([>\/~@#?])\s*(.*)$/.exec(raw);
    if (!m) return { mode: "all", q: raw.trim() };
    return { mode: { ">": "do", "/": "files", "~": "text", "@": "projects", "#": "conversations", "?": "ask" }[m[1]], q: m[2].trim() };
  }

  function paletteSearch(raw) {
    var p = palParse(raw || ""), seq = ++PAL.seq;
    clearTimeout(PAL.timer);
    var items = [];
    PAL.sources = [];
    PAL.note = "";
    if (p.mode === "ask") {
      items.push({ key: "ask", group: "Ask", title: p.q ? "Ask the assistant: " + p.q : "Type what you want done", detail: "opens a conversation; it asks before anything risky", run: function () { if (p.q) startChat(p.q, palContext()); } });
    } else if (!p.q && p.mode === "all") {
      palRecents().forEach(function (r) { items.push({ key: "r:" + r.key, group: "Recent", title: r.title, detail: r.detail, recent: r.key }); });
      items = items.concat(palActions("").slice(0, 5)).concat(palLocal("").filter(function (x) { return x.group === "Go to"; }).slice(0, 8));
      // recents replay by key against the live sources below
    } else {
      if (p.mode === "all" || p.mode === "do") items = items.concat(palLocal(p.q).filter(function (x) { return p.mode === "all" || x.group === "Do"; }));
    }
    state.palette.items = items; state.palette.index = 0; renderPalette();
    if (p.mode === "do" || p.mode === "ask" || (!p.q && p.mode === "all")) return;
    if (!p.q && p.mode !== "all") { PAL.note = ({ files: "Type part of a file name.", text: "Type the text to find in the current project.", projects: "Type a project name.", conversations: "Type part of a conversation title." })[p.mode] || ""; return renderPalette(); }
    PAL.timer = setTimeout(function () {
      var scopes = { all: ["projects", "files", "conversations", "models", "tasks"], files: ["files"], text: ["content"], projects: ["projects"], conversations: ["conversations"] }[p.mode];
      var args = { query: p.q, scopes: scopes, limit: 15 };
      if (p.mode === "text") { args.project = palContext(); if (!args.project) { PAL.note = "Text search needs a project. Add one first."; return renderPalette(); } }
      var caps = p.mode === "all" ? request("/api/capabilities?limit=8&q=" + encodeURIComponent(p.q)) : Promise.resolve(null);
      Promise.all([read("search.query", args), caps]).then(function (r) {
        if (seq !== PAL.seq) return;
        var more = palRemote(r[0]);
        if (r[1] && r[1].ok) (r[1].body.items || []).forEach(function (c) {
          more.push({ key: "cap:" + c.id, group: "Capabilities", title: c.id, detail: c.summary, score: 1, tier: c.tier, ask: c.requires_approval, run: function () {
            if (c.tier === "read" && !c.requires_approval) read(c.id, {}).then(function (d) { openDrawer(c.id, renderPatch(JSON.stringify(d, null, 2), 2000)); }).catch(function () { openCapability(c.id); });
            else openCapability(c.id);
          } });
        });
        more.sort(function (a, b) { return (b.score || 0) - (a.score || 0); });
        var all = state.palette.items.concat(more);
        // keep group order: Go to / Do first when they matched, then projects etc.
        var order = ["Do", "Go to", "Projects", "Sessions", "Services", "Sites", "Files", "Text in project", "Conversations", "Models", "Tasks", "Activity", "Capabilities"];
        all.sort(function (a, b) { return order.indexOf(a.group) - order.indexOf(b.group) || (b.score || 0) - (a.score || 0); });
        state.palette.items = all; PAL.sources = r[0].sources; state.palette.index = 0; renderPalette();
      }).catch(function (e) { if (seq === PAL.seq) { PAL.note = e.message; renderPalette(); } });
    }, 120);
  }

  function openPalette() {
    state.palette.open = true;
    el("palette").hidden = false;
    var input = el("palette-input");
    input.value = "";
    input.placeholder = "Search projects, files, conversations, models… or type > to do something, ? to ask";
    input.focus();
    paletteSearch("");
  }

  function closePalette() {
    state.palette.open = false;
    el("palette").hidden = true;
  }

  function renderPalette() {
    var items = state.palette.items;
    var list = el("palette-list");
    if (!items.length) {
      list.innerHTML = '<div class="empty">' + esc(PAL.note || (el("palette-input").value.trim() ? "Nothing matches. Prefix with ? to ask the assistant." : "Start typing")) + "</div>";
    } else {
      var last = "";
      list.innerHTML = items.map(function (c, i) {
        var head = c.group !== last ? '<div class="pi-head">' + esc(c.group) + "</div>" : "";
        last = c.group;
        return head + '<button type="button" class="palette-item' + (i === state.palette.index ? " active" : "") + '" data-action="palette-pick" data-index="' + i + '">' +
          '<span class="pi-main"><span class="pi-id">' + esc(c.title) + '</span><span class="pi-sum">' + esc(c.detail || "") + "</span></span>" +
          (c.ask ? '<span class="pill pill-warn">asks first</span>' : "") + (c.tier && c.tier !== "read" ? '<span class="pi-group">' + esc(c.tier) + "</span>" : "") + "</button>";
      }).join("");
      var active = list.querySelector(".palette-item.active");
      if (active && active.scrollIntoView) active.scrollIntoView({ block: "nearest" });
    }
    var src = (PAL.sources || []).map(function (s) {
      return s.searched ? s.source + (s.complete === false ? " (partial)" : "") : s.source + " (not searched: " + s.note + ")";
    }).join(" · ");
    el("palette-foot").innerHTML = '<span>↑↓ move</span><span>Enter open</span><span>Esc close</span>' + (src ? '<span class="grow nowrap" title="' + attr(src) + '">searched: ' + esc(src) + "</span>" : '<span class="grow dim nowrap">> do · / files · ~ text · @ projects · # conversations · ? ask</span>');
  }

  function paletteActivate(index) {
    var item = state.palette.items[index];
    if (!item) return;
    closePalette();
    if (item.recent) {
      // a recent entry is replayed as a search for the same words
      openPalette(); el("palette-input").value = item.title; return paletteSearch(item.title);
    }
    palRemember(item);
    if (item.run) item.run();
  }

  // -------------------------------------------------------------------- views

  // The sidebar answers "what am I doing": things that need me, the work, the
  // machine it runs on, and the settings behind it. Related pages share one
  // entry and appear as tabs, so the rail stays short.
  var AREA = {
    assistant: "assistant", team: "agents",
    projects: "projects", project: "projects", create: "projects", develop: "projects",
    sites: "sites", site: "sites", hosts: "sites", certificates: "sites",
    agents: "agents", schedules: "agents", shared: "agents", skills: "agents", mcp: "agents",
    overview: "server", services: "server", operations: "server", toolchain: "server", runtimes: "server",
    databases: "server", remote: "server", preview: "server"
  };
  var TABS = {
    projects: [["projects", "Projects"], ["create", "New project"], ["develop", "Stacks and tasks"]],
    sites: [["sites", "All sites"], ["hosts", "Hosts"], ["certificates", "Certificates"]],
    agents: [["team", "Runs"], ["agents", "Team"], ["schedules", "Schedules"], ["skills", "Skills"], ["shared", "Notes"], ["mcp", "MCP"]],
    server: [["overview", "Overview"], ["services", "Services"], ["runtimes", "Runtimes"], ["databases", "Databases"], ["toolchain", "Tools"], ["remote", "Remote"]]
  };
  var AREA_HOME = { assistant: "assistant", projects: "projects", sites: "sites", agents: "team", server: "overview" };

  function areaOf(view) { return AREA[view] || view; }

  var TAB_ICON = { projects: "i-folder", create: "i-plus", develop: "i-wrench", sites: "i-globe", hosts: "i-server", certificates: "i-lock", team: "i-team", agents: "i-layers", schedules: "i-calendar", skills: "i-book", shared: "i-file", mcp: "i-code", overview: "i-home", services: "i-layers", runtimes: "i-bolt", databases: "i-db", toolchain: "i-wrench", remote: "i-cloud" };

  function subnav() {
    var area = AREA[state.view];
    var tabs = area && TABS[area];
    var box = el("subnav");
    if (!tabs || state.view === "project" || state.view === "site") { box.hidden = true; box.innerHTML = ""; return; }
    box.hidden = false;
    box.innerHTML = tabs.map(function (t) {
      var active = state.view === t[0] || (t[0] === "sites" && state.view === "site");
      return '<button type="button" class="subtab' + (active ? " active" : "") + '" data-action="nav" data-view="' + t[0] + '">' + (TAB_ICON[t[0]] ? '<svg class="icon" aria-hidden="true"><use href="#' + TAB_ICON[t[0]] + '"></use></svg>' : "") + esc(t[1]) + "</button>";
    }).join("");
  }

  var RAIL_ICON = {
    home: "i-home", inbox: "i-inbox", assistant: "i-spark", projects: "i-folder",
    sites: "i-globe", agents: "i-team", team: "i-team", processes: "i-pulse", overview: "i-server",
    providers: "i-sliders", activity: "i-clock", system: "i-check-circle", capabilities: "i-code"
  };

  // ------------------------------------------------- the PHP processes are one thing
  //
  // php-cgi answers one request at a time, so each PHP version runs as a small pool: php-cgi-8.4,
  // php-cgi-8.4-2 and so on. To a person that is "PHP 8.4", running or not, so the screens show one row
  // for the pool and count it once.

  function phpPoolOf(name) {
    var m = /^php-cgi-(\d+\.\d+)(-alt)?(?:-(\d+))?$/.exec(name || "");
    return m ? { series: m[1], alt: !!m[2] } : null;
  }

  /* The services as the person sees them: each ordinary service, and one unit per PHP pool. */
  function serviceUnits(rows) {
    var units = [], groups = {};
    (rows || []).forEach(function (r) {
      var pool = phpPoolOf(r.name);
      if (!pool) { units.push({ row: r, running: !!r.running }); return; }
      var key = pool.series + (pool.alt ? "-alt" : "");
      if (!groups[key]) { groups[key] = { php: pool, rows: [] }; units.push(groups[key]); }
      groups[key].rows.push(r);
    });
    units.forEach(function (u) { if (u.rows) u.running = u.rows.length > 0 && u.rows.every(function (r) { return r.running; }); });
    return units;
  }

  function serviceCounts(rows) {
    var units = serviceUnits(rows);
    return { total: units.length, up: units.filter(function (u) { return u.running; }).length };
  }

  /* The web server is chosen with web.use, and PHP listeners follow the
     installed runtimes. The rest are a person's choice, recorded here. */
  function startupToggle(svc) {
    if (!svc.installed || svc.kind === "php-cgi" || svc.name === "nginx" || svc.name === "apache") return "";
    return '<label class="check svc-startup" title="Include this service when you press Start all or devd starts with your account. It does not start it now.">' +
      '<input type="checkbox" data-change="service-startup" data-name="' + attr(svc.name) + '"' + (svc.enabled ? " checked" : "") + "> Startup</label>";
  }

  /* What leans on a service, read from the site list and nothing else. A web server
     serves the sites that have something to serve; a PHP pool serves the sites that
     resolve to its version. A database has no recorded dependents, so it says none. */
  function serviceDependents(svc, php) {
    var sites = state.server.sites || [];
    var serving = sites.filter(function (r) { return r.serving !== "no_entry"; });
    if (php) {
      var mine = serving.filter(function (r) { return String(r.php_effective || "").indexOf(php) === 0; });
      return mine.length ? "serves " + mine.length + (mine.length === 1 ? " site" : " sites") + ": " + mine.slice(0, 3).map(function (r) { return r.site.host || r.site.name; }).join(", ") + (mine.length > 3 ? ", and " + (mine.length - 3) + " more" : "") : "no site resolves to this version";
    }
    if (svc.name === "nginx" || svc.name === "apache") {
      var web = (state.session && state.session.settings && state.session.settings.web_server) || "nginx";
      if (svc.name !== web) return "not the web server chosen for sites";
      var needsPhp = {};
      serving.forEach(function (r) { if (r.serving === "php") needsPhp[r.php_effective || "?"] = 1; });
      var keys = Object.keys(needsPhp);
      return "serves " + serving.length + (serving.length === 1 ? " site" : " sites") + (keys.length ? "; needs PHP " + keys.join(", ") : "");
    }
    if (svc.kind === "mariadb" || svc.kind === "mysql" || svc.kind === "postgres") return "no site is recorded as depending on it";
    return "";
  }

  /* Where a service is on the way from files on disk to doing its job. Each rung
     is a separate fact: installed is not running, and a process holding its port
     is not a service that answers. The words carry the meaning; the dot repeats it. */
  function serviceLadder(svc) {
    if (!svc.installed) return { dot: "bad", facts: ['<span class="fact fact-bad">files missing</span>'], word: "files missing" };
    if (!svc.running && svc.listening) {
      // devd did not start what is on this port. It may be another copy of devd, a
      // system service or something left over; it is reported, never adopted.
      return { dot: "warn", external: true,
        facts: ['<span class="fact">installed</span>', '<span class="fact fact-warn">not started by devd</span>',
          '<span class="fact" title="' + attr(svc.ready_detail || "") + '">port ' + esc(svc.port) + (svc.ready ? " answers" : " in use") + "</span>"],
        word: "another program holds its port",
        detail: "held by a process devd did not start" + (svc.ready_detail ? " (it " + svc.ready_detail + ")" : "") + "; starting this one would collide with it" };
    }
    if (!svc.running) {
      var want = svc.desired_state === "running";
      return { dot: want ? "warn" : "off", facts: ['<span class="fact">installed</span>', want ? '<span class="fact fact-warn">should be running</span>' : '<span class="fact">stopped</span>'], word: want ? "should be running" : "stopped" };
    }
    var facts = ['<span class="fact">installed</span>', '<span class="fact fact-ok">running</span>'];
    if (svc.ready === true) {
      facts.push('<span class="fact fact-ok" title="' + attr(svc.ready_detail || "") + '">answers' + (svc.ready_via && svc.ready_via !== "tcp" ? " (" + esc(svc.ready_via) + ")" : "") + "</span>");
      return { dot: "ok", facts: facts, word: svc.ready_via === "tcp" ? "accepts connections" : "answers" };
    }
    if (svc.ready === false) {
      facts.push('<span class="fact fact-warn" title="' + attr(svc.ready_detail || "") + '">not answering</span>');
      return { dot: "warn", facts: facts, word: "running but not answering", detail: svc.ready_detail };
    }
    return { dot: "ok", facts: facts, word: "running" };
  }

  function phpUnitRow(u, compact) {
    var n = u.rows.length;
    var up = u.rows.filter(function (r) { return r.running; }).length;
    var ports = u.rows.map(function (r) { return r.port; }).filter(Boolean).join(", ");
    var tag = (u.php.alt ? "other limits profile · " : "") + n + (n === 1 ? " worker" : " workers");
    var answering = u.rows.filter(function (r) { return r.ready === true; }).length;
    var silent = u.rows.filter(function (r) { return r.running && r.ready === false; }).length;
    var held = u.rows.filter(function (r) { return !r.running && r.listening; }).length;
    var series = attr(u.php.series);
    var facts = [up === n ? '<span class="fact fact-ok">running</span>' : up ? '<span class="fact fact-warn">' + up + " of " + n + " running</span>" : '<span class="fact">stopped</span>'];
    if (up && silent) facts.push('<span class="fact fact-warn">' + silent + " not answering</span>");
    else if (up && answering >= up) facts.push('<span class="fact fact-ok">answers (fastcgi)</span>');
    if (held) facts.push('<span class="fact fact-warn" title="Something devd did not start is listening on these ports">' + held + (held === 1 ? " port" : " ports") + " held elsewhere</span>");
    return uiRow({
      dot: up === n && !silent ? "ok" : up || held ? "warn" : "off",
      title: brandImg("php-cgi", "brand-dot") + '<span class="row-name">PHP ' + esc(u.php.series) + '</span> <span class="fact mono">' + esc(tag) + "</span>" +
        ' <span class="facts">' + facts.join("") + "</span>",
      sub: (compact ? "" : (serviceDependents(null, u.php.series) ? '<div class="ui-row-dep">' + esc(serviceDependents(null, u.php.series)) + "</div>" : "")) +
        (compact || !ports ? "" : '<span class="mono">ports ' + esc(ports) + "</span>") +
        (held && !up ? (compact || !ports ? "" : " · ") + esc("devd did not start what is on " + (held === n ? "these ports" : "some of these ports") + "; starting the pool would collide with it") : ""),
      wrap: !!held,
      actions: (up
        ? '<button type="button" class="btn btn-mini" data-action="php-pool" data-do="stop" data-series="' + series + '">Stop</button>' +
          '<button type="button" class="btn btn-mini btn-ghost" data-action="php-pool" data-do="restart" data-series="' + series + '">Restart</button>'
        : '<button type="button" class="btn btn-mini btn-primary" data-action="php-pool" data-do="start" data-series="' + series + '">Start</button>') +
        (compact ? "" : '<button type="button" class="btn btn-mini btn-ghost" data-action="service-logs" data-name="' + attr(u.rows[0].name) + '">Logs</button>'),
      hover: !compact
    });
  }

  /* The command bar.
   *
   * Twelve destinations are not twelve equals. Six are where work happens every
   * day and are always in view, in the order a change travels: look at the
   * project, its address, what runs, then ask for something, then watch it run.
   * Five are the machine and its records. They are one step away, under two
   * names, and reachable from the keyboard with Ctrl K. What needs a person is
   * not a destination among the others: it sits apart, at the end, and speaks
   * only when there is something to decide. */
  function rail() {
    var server = state.server;
    var counts = serviceCounts(server.services);
    var attention = (AI.counts.approvals + AI.counts.questions) || null;
    var working = AI.threads ? (AI.threads.filter(function (t) { return t.state === "running"; }).length || null) : null;
    var runs = (state.view === "team" && TEAM.runs && TEAM.runs.length) ? TEAM.runs : (AI.liveRuns || (AI.home && AI.home.teamRuns) || []);
    var live = runs.filter(function (t) { return ["planning", "running"].indexOf(t.state) >= 0; });
    var groups = [
      { label: "Work", items: [
        { view: "home", label: "Home" },
        { view: "projects", label: "Projects", count: state.projects.length || state.projectCount || null },
        { view: "sites", label: "Sites", count: (server.sites || []).length || null },
        { view: "processes", label: "Processes" }
      ] },
      { label: "Agents", items: [
        { view: "assistant", label: "Assistant", count: working },
        // Named for one screen, so it always opens that screen, not the last tab used.
        { view: "team", label: "Agent runs", count: live.length || null, exact: true }
      ] },
      { label: "Machine", menu: true, items: [
        { view: "overview", label: "Environment", hint: "services, runtimes, databases, hostnames", count: counts.total ? counts.up + "/" + counts.total + " running" : null },
        { view: "providers", label: "Models", hint: "providers, defaults and fallbacks" },
        { view: "capabilities", label: "API and tools", hint: "what agents are able to call" }
      ] },
      { label: "Records", menu: true, items: [
        { view: "activity", label: "Activity", hint: "everything devd did, and who asked" },
        { view: "system", label: "Diagnostics", hint: "health checks and what to do about them" }
      ] }
    ];
    function tab(i) {
      var active = areaOf(state.view) === areaOf(i.view);
      return '<button type="button" class="top-tab' + (active ? " active" : "") + '" data-action="nav" data-view="' + i.view + '"' + (i.exact ? ' data-exact="1"' : "") + (active ? ' aria-current="page"' : "") + ">" +
        esc(i.label) + (i.count !== null && i.count !== undefined ? '<span class="top-count">' + esc(i.count) + "</span>" : "") + "</button>";
    }
    var html = groups.map(function (group, gi) {
      if (!group.menu) return '<div class="top-group" role="group" aria-label="' + attr(group.label) + '">' + group.items.map(tab).join("") + "</div>";
      var open = state.topMenu === group.label;
      var here = group.items.some(function (i) { return areaOf(state.view) === areaOf(i.view); });
      return '<div class="top-menu"><button type="button" class="top-tab top-tab-menu' + (here ? " active" : "") + '" data-action="top-menu" data-menu="' + attr(group.label) + '" aria-haspopup="true" aria-expanded="' + open + '">' +
        esc(group.label) + '<svg class="icon" aria-hidden="true"><use href="#i-chevron-down"></use></svg></button>' +
        '<div class="top-pop" role="group" aria-label="' + attr(group.label) + '"' + (open ? "" : " hidden") + ">" + group.items.map(function (i) {
          var on = areaOf(state.view) === areaOf(i.view);
          return '<button type="button" class="top-item' + (on ? " active" : "") + '" data-action="nav" data-view="' + i.view + '"' + (on ? ' aria-current="page"' : "") + ">" +
            '<span class="top-item-name">' + esc(i.label) + "</span>" +
            '<span class="top-item-hint">' + esc(i.hint) + (i.count ? " · " + esc(i.count) : "") + "</span></button>";
        }).join("") + "</div></div>";
    }).join("");

    // Work in flight, named as it is: the run's own title and its real state.
    var chip = "";
    if (live.length) {
      var first = live[0];
      var title = String(first.title || "").length > 34 ? String(first.title).slice(0, 33) + "…" : String(first.title || "Agent run");
      chip = '<button type="button" class="top-live" data-action="team-open-from" data-id="' + attr(first.id) + '" title="' + attr(first.title || "") + '">' +
        '<span class="top-live-state">' + esc(first.state === "planning" ? "Planning" : "Running") + "</span>" +
        '<span class="top-live-title">' + esc(title) + "</span>" +
        (live.length > 1 ? '<span class="top-count">+' + (live.length - 1) + "</span>" : "") + "</button>";
    }
    var needs = '<button type="button" class="top-needs' + (attention ? " has" : "") + (areaOf(state.view) === "inbox" ? " active" : "") + '" data-action="nav" data-view="inbox"' + (areaOf(state.view) === "inbox" ? ' aria-current="page"' : "") + ">" +
      (attention ? '<span class="top-needs-n">' + esc(attention) + "</span>" : "") + "Needs you</button>";
    el("rail").innerHTML = html;
    el("top-attn").innerHTML = chip + needs;
  }

  /* The path above the page, only when it says something the title and the tabs do not: a project is
   * "Projects / name". A page's own name is already the title, and the tab, and the rail entry. */
  function crumbs() {
    var html = "";
    if (state.view === "project" && state.project) {
      html = '<button type="button" data-action="nav" data-view="projects">Projects</button>' +
        '<span class="sep">/</span><span>' + esc(state.project.name) + "</span>";
    }
    if (state.view === "site" && SITE.name) {
      html = '<button type="button" data-action="nav" data-view="sites">Sites</button>' +
        '<span class="sep">/</span><span>' + esc(SITE.name) + "</span>";
    }
    el("crumbs").innerHTML = html;
  }

  function statusbar() {
    var left = [], right = [];
    if (state.project && state.project.status && state.view === "project") {
      var st = state.project.status;
      left.push("<b>" + esc(state.project.name) + "</b> on " + esc(st.branch || "(detached)"));
      var dirty = st.staged.length + st.unstaged.length + st.untracked.length;
      left.push(dirty ? dirty + " changed" : "clean");
      if (st.conflicted.length) left.push('<span class="sb-bad">' + st.conflicted.length + " conflicted</span>");
      if (st.ahead) left.push(st.ahead + " ahead");
      if (st.behind) left.push(st.behind + " behind");
    }
    if (state.server.loaded) {
      var counts = serviceCounts(state.server.services);
      var total = counts.total;
      var up = counts.up;
      if (total) left.push(up + "/" + total + " services up");
    }
    var chats = AI.threads ? AI.threads.filter(function (t) { return t.state === "running"; }).length : 0;
    if (chats) left.push(chats + " agent" + (chats === 1 ? "" : "s") + " working");
    var wait = AI.counts.approvals + AI.counts.questions;
    if (wait) left.push('<button type="button" class="sb-link sb-warn" data-action="nav" data-view="inbox">' + wait + " waiting for you</button>");
    var prod = AI.env && AI.env.mode === "production";
    right.push(prod ? '<span class="sb-bad" title="Production mode: agents need your approval for every change">production</span>' : '<span title="devd only changes this machine. Switch to production mode in Diagnostics to require approval for every agent change.">local</span>');
    if (state.session) right.push('<span title="The address agents and scripts use">api :' + esc(state.session.port) + "</span>");
    el("statusbar").innerHTML = '<div class="sb-left"><button type="button" class="health health-checking" id="health-chip" data-action="health-open" title="System health"></button><span class="sb-sep" aria-hidden="true">·</span>' + (left.join('<span class="sb-sep" aria-hidden="true">·</span>') || "Ready") + '</div><div class="sb-right">' + right.join('<span class="sb-sep" aria-hidden="true">·</span>') + "</div>";
    healthChip();
  }

  // -------------------------------------------------------------------- server

  var SERVER_VIEWS = ["overview", "sites", "site", "services", "toolchain", "runtimes", "databases", "remote", "operations", "preview", "certificates", "hosts"];

  var KNOWN_SERVICES = [
    { name: "nginx", label: "nginx", note: "web server" },
    { name: "apache", label: "Apache", note: "reads .htaccess; use the one you already have", find: true, download: true },
    { name: "mariadb", label: "MariaDB", note: "database" },
    { name: "redis", label: "Redis", note: "cache, queues and sessions; downloaded and checked, or the one you already have", find: true, download: true },
    { name: "postgres", label: "PostgreSQL", note: "uses the one you already have", find: true, download: false },
    { name: "mongodb", label: "MongoDB", note: "uses the one you already have", find: true, download: false },
    { name: "memcached", label: "Memcached", note: "object cache; downloaded and checked, or the one you already have", find: true, download: true },
    { name: "mailpit", label: "Mailpit", note: "catches the mail your applications send" },
    { name: "php-cgi", label: "php-cgi", note: "PHP FastCGI listener, built from the installed runtime" }
  ];

  var RUNTIME_KINDS = [
    { kind: "php", label: "PHP", note: "8.x, downloaded from windows.php.net" },
    { kind: "node", label: "Node.js", note: "LTS and current releases" },
    { kind: "go", label: "Go", note: "the official toolchain" },
    { kind: "python", label: "Python", note: "standalone CPython builds, with pip and venv" },
    { kind: "ruby", label: "Ruby", note: "RubyInstaller builds (Windows)" }
  ];

  /* The heading of a page: its name. */
  function pageTitle(text) {
    return '<h1 class="page-title">' + esc(text) + "</h1>";
  }

  function page(title, sub, actions, body) {
    return (
      '<div class="page">' +
      '<div class="page-head"><div class="page-heading">' +
      pageTitle(title) + (sub ? '<div class="page-sub">' + esc(sub) + "</div>" : "") + "</div>" +
      '<div class="page-actions">' + (actions || "") + "</div>" +
      "</div>" + body + "</div>"
    );
  }

  // ---------------------------------------------------------- calm components
  //
  // The vocabulary every screen is written in: a row, a status dot, a section
  // head, an empty state and a sheet that slides in for the detail. Titles and
  // subtitles are HTML the caller has already escaped.

  function uiDot(kind) {
    return '<span class="ui-dot ui-dot-' + (kind || "off") + '" aria-hidden="true"></span>';
  }

  /* o: { dot, title, sub, wrap, meta, actions, hover, accent, open (attrs), mono } */
  function uiRow(o) {
    var attrs = o.open ? ' data-open="1" ' + o.open : "";
    return '<div class="ui-row' + (o.accent ? " ui-row-accent" : "") + (o.mono ? " mono-title" : "") + '"' + attrs + ">" +
      (o.dot ? uiDot(o.dot) : "") +
      '<div class="ui-row-main"><div class="ui-row-title">' + o.title + "</div>" +
      (o.sub ? '<div class="ui-row-sub' + (o.wrap ? " wrap" : "") + '">' + o.sub + "</div>" : "") + "</div>" +
      (o.meta ? '<div class="ui-row-meta">' + o.meta + "</div>" : "") +
      (o.actions ? '<div class="ui-row-actions' + (o.hover ? " hover" : "") + '">' + o.actions + "</div>" : "") +
      "</div>";
  }

  function uiList(rows) {
    return rows.length ? '<div class="ui-list">' + rows.join("") + "</div>" : "";
  }

  function uiHead(title, count, actions) {
    return '<div class="ui-head"><h2>' + esc(title) + "</h2>" +
      (count !== null && count !== undefined && count !== "" ? '<span class="count">' + esc(String(count)) + "</span>" : "") +
      '<span class="grow"></span>' + (actions || "") + "</div>";
  }

  function uiEmpty(title, hint, action) {
    return '<div class="ui-empty"><b>' + esc(title) + "</b>" + (hint ? "<span>" + esc(hint) + "</span>" : "") + (action || "") + "</div>";
  }

  /* A compact ledger of the state of the environment.
   *
   * It is one bordered surface with a few facts, not a grid of dashboard cards.
   * A value is always paired with its label, and a warning carries its own
   * explanation, so a colour is never the only thing that communicates state. */
  function summaryBand(items) {
    return '<div class="summary-band">' + items.map(function (i) {
      return '<div class="summary-item' + (i.tone ? " summary-" + i.tone : "") + '">' +
        '<div class="summary-label">' + (i.dot ? uiDot(i.dot) : "") + esc(i.label) + "</div>" +
        '<div class="summary-value">' + esc(i.value) + "</div>" +
        (i.detail ? '<div class="summary-detail">' + esc(i.detail) + "</div>" : "") +
        "</div>";
    }).join("") + "</div>";
  }

  /* One readable line from a long report: no markdown, no truncation markers,
   * the first sentence, never more than `max` characters. */
  function uiSummary(text, max) {
    var t = String(text || "").replace(/\[truncated[^\]]*\]/g, "").replace(/[`*_#>]/g, "").replace(/\s+/g, " ").trim();
    var m = t.match(/^(.{20,}?[.!?])(\s|$)/);
    var s = m ? m[1] : t;
    max = max || 150;
    return s.length > max ? s.slice(0, max - 1).trimEnd() + "…" : s;
  }

  /* The full text of a stored report, without the markers the store left. */
  function uiBody(text) {
    return md(String(text || "").replace(/\s*\[truncated[^\]]*\]/g, ""));
  }

  function uiSheet(title, html) {
    openDrawer(title, '<div class="ui-sheet">' + html + "</div>");
  }

  function dot(on, warn) {
    return '<span class="dot ' + (on ? "dot-ok" : warn ? "dot-warn" : "dot-off") + '" aria-hidden="true"></span>';
  }

  /* A box that narrows a list.
   *
   * Every list on the server screens can be searched. The filter narrows what
   * is on screen and nothing else, so a service that is not on screen is not
   * stopped, and the count beside a heading always says what is really there.
   */
  function filterBox(key, placeholder) {
    var v = state.filters[key] || "";
    return '<input type="search" class="ui-search" data-filter="' + key + '" value="' + attr(v) + '" placeholder="' + attr(placeholder) + '" aria-label="' + attr(placeholder) + '" spellcheck="false">';
  }

  /* Whether a row survives the filter. */
  function keeps(query, ...fields) {
    var q = String(query || "").trim().toLowerCase();
    if (!q) return true;
    var hay = fields.filter(Boolean).join(" ").toLowerCase();
    return q.split(/\s+/).every(function (word) { return hay.indexOf(word) !== -1; });
  }

  function statCard(label, value, sub) {
    return (
      '<div class="stat">' +
      '<div class="stat-label">' + esc(label) + "</div>" +
      '<div class="stat-value">' + esc(value) + "</div>" +
      '<div class="stat-sub">' + esc(sub) + "</div>" +
      "</div>"
    );
  }

  function notice(text, label, action, view) {
    return (
      '<div class="notice">' +
      '<div class="grow">' + esc(text) + "</div>" +
      '<button type="button" class="btn btn-mini' + (action === "ca-trust" ? " btn-primary" : "") + '" data-action="' + action + '"' +
      (view ? ' data-view="' + view + '"' : "") + ">" + esc(label) + "</button>" +
      "</div>"
    );
  }

  function servicePorts(svc) {
    if (svc.kind === "nginx" && state.session && state.session.settings) {
      return "http " + state.session.settings.http_port + " · https " + state.session.settings.https_port;
    }
    return svc.port ? "port " + svc.port : "";
  }

  function phpRuntime() {
    var php = state.server.runtimes.filter(function (r) { return r.runtime.kind === "php"; });
    // What devd installed and marked as the default comes first. Then any
    // runtime devd installed. Then one that was already on the machine: it
    // is real and usable, and saying "not installed" because devd did not
    // put it there would be a lie.
    var primary = php.filter(function (r) { return r.primary; })[0];
    if (primary) return primary;
    var managed = php.filter(function (r) { return r.managed !== false; })[0];
    if (managed) return managed;
    return php[0] || null;
  }

  /* The release series of a version: 8.3.14 is series 8.3. */
  function devdSeries(v) {
    return String(v || "").split(".").slice(0, 2).join(".");
  }

  function phpPrimary() {
    var r = phpRuntime();
    return r ? r.runtime.version : null;
  }

  /* Whether the operating system trusts devd's authority, as reported by the
   * server. The check does not change anything. */
  function caTrusted() {
    var t = state.server.trust;
    return !!(t && (t.user || t.machine));
  }

  function caTrustLabel() {
    var t = state.server.trust;
    if (t && t.machine) return "trusted machine-wide";
    if (t && t.user) return "trusted for this user";
    return "not trusted yet";
  }

  /* The reason a server read failed, or "" when it did not. */
  function serverFailed(id) {
    return (state.server.failures && state.server.failures[id]) || "";
  }

  function loadServer() {
    // The inventory probes every installed service and port, which takes
    // seconds. The screen does not wait for it: services, sites and runtimes
    // are enough to draw the page, and the inventory panel appears when its
    // answer arrives.
    //
    // The same reads are needed by the rail, Home and several screens during
    // one boot. They share one request instead of each asking the machine
    // again; a second caller joins the answer that is already on its way.
    if (state.server.pending) return state.server.pending;
    // A read that fails is not an empty answer. Its last value is kept and the
    // failure recorded, so no screen says "no sites" because a request was refused.
    var FAILED = {};
    var failures = {};
    function attempt(id) {
      return read(id).catch(function (e) { failures[id] = e && e.message ? e.message : "could not be read"; return FAILED; });
    }
    var request = Promise.all([
      attempt("service.list"), attempt("site.list"), attempt("runtime.list"), attempt("tls.list"),
      attempt("tls.ca.show"), attempt("hosts.show"), attempt("tls.ca.status")
    ]).then(function (r) {
      function take(i, key, list) {
        if (r[i] === FAILED) { if (state.server[key] === undefined) state.server[key] = list ? [] : null; return; }
        state.server[key] = list ? (Array.isArray(r[i]) ? r[i] : []) : r[i];
      }
      take(0, "services", true);
      take(1, "sites", true);
      take(2, "runtimes", true);
      take(3, "certs", true);
      take(4, "ca", false);
      take(5, "hosts", false);
      take(6, "trust", false);
      state.server.failures = failures;
      state.server.loaded = true;
      read("services.inventory")
        .then(function (inv) {
          state.server.inventory = inv && inv.services ? inv.services : [];
          if (SERVER_VIEWS.indexOf(state.view) !== -1 || state.view === "home") render();
        })
        .catch(function () { /* the panel simply stays out of the page */ });
    });
    state.server.pending = request.then(
      function (value) { state.server.pending = null; return value; },
      function (error) { state.server.pending = null; throw error; }
    );
    return state.server.pending;
  }

  // ------------------------------------------------------------------ overview

  function loadMachine() {
    if (MACHINE.loading || MACHINE.data) return Promise.resolve();
    MACHINE.loading = true;
    MACHINE.error = "";
    return read("monitor.stats", {})
      .then(function (d) { MACHINE.data = d; })
      .catch(function (e) { MACHINE.error = e.message; })
      .then(function () {
        MACHINE.loading = false;
        if (state.view === "overview") render();
      });
  }

  function machineSummary(m) {
    var mem = m.memory || {};
    var parts = [
      m.cpu_percent != null ? "CPU " + Math.round(m.cpu_percent) + "%" : (m.load_average != null ? "load " + m.load_average : ""),
      mem.total_mb ? "memory " + Math.round(mem.used_mb / 1024 * 10) / 10 + " of " + Math.round(mem.total_mb / 1024 * 10) / 10 + " GB" : "",
      m.uptime_hours != null ? "up " + m.uptime_hours + " h" : "",
      m.devd ? m.devd.running + "/" + m.devd.services + " devd services" : ""
    ].filter(Boolean);
    var disks = (m.disks || []).map(function (x) { return esc(x.mount) + " " + esc(String(x.free_gb)) + " GB free of " + esc(String(x.total_gb)) + " GB"; }).join(" \u00b7 ");
    return uiHead("Machine", null, "") + uiList([
      uiRow({ dot: "off", title: "This machine", sub: esc(parts.join(" \u00b7 ")), wrap: true }),
      disks ? uiRow({ dot: "off", title: "Disks", sub: disks, wrap: true }) : ""
    ].filter(Boolean));
  }

  /* The path a request travels to reach a site on this machine, as five links:
     the name resolves, the certificate is trusted, the web server answers, PHP is
     there to run the code, and the database is there to hold it. Each link states
     what devd measured and leads to where it is changed. A link devd has no
     reading for says so; none is assumed to be fine. */
  function machineChain() {
    var s = state.server;
    var hosts = s.hosts || {};
    var pending = ((hosts.adding || []).length + (hosts.removing || []).length);
    var sites = s.sites || [];
    var published = sites.filter(function (r) { return r.in_hosts; }).length;
    var web = (state.session && state.session.settings && state.session.settings.web_server) || "nginx";
    var webSvc = (s.services || []).filter(function (r) { return r.name === web; })[0];
    var pools = (s.services || []).filter(function (r) { return /^php-cgi/.test(r.name) && r.name !== "php-cgi"; });
    var poolsUp = pools.filter(function (r) { return r.running; }).length;
    var db = (s.services || []).filter(function (r) { return r.kind === "mariadb" || r.kind === "mysql"; })[0];
    var runtimes = s.runtimes || [];

    function link(view, title, tone, word, detail) {
      return '<button type="button" class="chain-link chain-' + tone + '" data-action="nav" data-view="' + view + '">' +
        '<span class="chain-title">' + esc(title) + '</span><span class="chain-word">' + esc(word) + '</span><span class="chain-detail">' + esc(detail) + "</span></button>";
    }
    var webL = webSvc ? serviceLadder(webSvc) : null;
    var dbL = db ? serviceLadder(db) : null;
    var tone = function (dot) { return dot === "ok" ? "ok" : dot === "bad" ? "bad" : dot === "warn" ? "warn" : "off"; };
    var links = [
      link("hosts", "Names", !sites.length ? "off" : pending || published < sites.length ? "warn" : "ok",
        !sites.length ? "No sites" : published + " of " + sites.length + " resolve", pending ? pending + " change" + (pending === 1 ? "" : "s") + " waiting to be written" : "devd's block in the hosts file"),
      link("certificates", "HTTPS", !s.ca ? "off" : caTrusted() ? "ok" : "warn",
        !s.ca ? "No authority yet" : caTrusted() ? "Trusted" : "Not trusted", (s.certs || []).length + " certificate" + ((s.certs || []).length === 1 ? "" : "s") + " issued"),
      link("services", "Web server", webL ? tone(webL.dot) : "off", webL ? webL.word : "Not installed", web + (webSvc && webSvc.port ? " on port " + webSvc.port : "")),
      link("runtimes", "PHP", !runtimes.length ? "off" : pools.length && poolsUp === pools.length ? "ok" : "warn",
        !runtimes.length ? "No runtime" : pools.length ? poolsUp + " of " + pools.length + " pools running" : runtimes.length + " installed", phpRuntime() ? "default " + phpPrimary() : "no default chosen"),
      link("databases", "Database", dbL ? tone(dbL.dot) : "off", dbL ? dbL.word : "Not installed", db ? db.name + (db.port ? " on port " + db.port : "") : "install one when a project needs it")
    ];
    return '<div class="chain" aria-label="The path a request takes on this machine">' + links.join('<span class="chain-sep" aria-hidden="true"></span>') + "</div>";
  }

  function renderOverview() {
    if (state.view !== "overview") return;
    var s = state.server;
    var counts = serviceCounts(s.services);
    var total = counts.total;
    var up = counts.up;
    var hosts = s.hosts || {};
    var pending = (hosts.adding || []).length + (hosts.removing || []).length;

    if (!MACHINE.data && !MACHINE.loading && !MACHINE.error) loadMachine();
    var machine = MACHINE.data ? machineSummary(MACHINE.data) : "";
    var actions = total
      ? '<button type="button" class="btn" data-action="stack-down"' + (up ? "" : " disabled") + '>Stop all</button><button type="button" class="btn btn-primary" data-action="stack-up">Start all</button>'
      : '<button type="button" class="btn btn-primary" data-action="setup-open">Set up devd</button>';

    var notices = [];
    if (pending) {
      notices.push(uiRow({ accent: true, dot: "warn", title: pending + (pending === 1 ? " hostname is" : " hostnames are") + " not published",
        sub: "Sites on ." + esc((state.session && state.session.settings && state.session.settings.tld) || "test") + " will not resolve until the hosts file is updated.",
        actions: '<button type="button" class="btn btn-primary btn-mini" data-action="nav" data-view="hosts">Review</button>' }));
    }
    if (s.certs.length && !caTrusted()) {
      notices.push(uiRow({ accent: true, dot: "warn", title: "Trust devd's certificate authority", sub: "Browsers will stop warning about your local https sites.",
        actions: '<button type="button" class="btn btn-primary btn-mini" data-action="ca-trust">Trust it</button>' }));
    }

    var noEntry = (s.sites || []).filter(function (row) { return row.serving === "no_entry"; });
    var siteLine = uiRow({
      dot: noEntry.length ? "warn" : s.sites.length ? "ok" : "off",
      title: s.sites.length + (s.sites.length === 1 ? " site" : " sites") + " · " + (s.sites.length - noEntry.length) + " can serve",
      sub: noEntry.length
        ? noEntry.length + (noEntry.length === 1 ? " site has" : " sites have") + " no web entry point. The address still exists; the folder has nothing to serve."
        : s.sites.length ? "Every site has a defined entry point. Open Sites to inspect an address, serving mode or certificate." : "A site is a folder, a hostname and a certificate.",
      actions: '<button type="button" class="btn btn-mini btn-primary" data-action="nav" data-view="sites">Sites</button>'
    });
    var answeredCount = (s.services || []).filter(function (x) { return x.running && x.ready === true; }).length;
    var silentCount = (s.services || []).filter(function (x) { return x.running && x.ready === false; }).length;
    var runningRows = (s.services || []).filter(function (x) { return x.running; }).length;
    var heldCount = (s.services || []).filter(function (x) { return !x.running && x.listening; }).length;
    var services = serviceUnits(s.services).map(function (unit) {
      if (unit.rows) return phpUnitRow(unit, true);
      var svc = unit.row;
      var ladder = serviceLadder(svc);
      return uiRow({
        dot: ladder.dot,
        title: brandImg(svc.kind || svc.name, "brand-dot") + '<span class="row-name">' + esc(svc.name) + "</span>" +
          ' <span class="facts">' + ladder.facts.join("") + "</span>",
        sub: '<span class="mono">' + esc(servicePorts(svc) || "") + "</span>" + (ladder.detail ? " · " + esc(ladder.detail) : ""),
        actions: svc.running
          ? '<button type="button" class="btn btn-mini" data-action="stop-service" data-name="' + attr(svc.name) + '">Stop</button>'
          : '<button type="button" class="btn btn-mini btn-primary" data-action="start-service" data-name="' + attr(svc.name) + '">Start</button>'
      });
    });

    el("main").innerHTML = page("Environment", "", actions,
      (notices.length ? uiList(notices) : "") +
      machineChain() +
      siteLine + machine +
      uiHead("Services", total ? up + " of " + total : null) +
      (services.length ? uiList(services) : serverFailed("service.list") ? uiEmpty("Couldn't read your services", serverFailed("service.list") + ".", '<button type="button" class="btn" data-action="refresh">Try again</button>') : uiEmpty("No services installed", "devd installs PHP, a web server and MariaDB from their makers.", '<button type="button" class="btn btn-primary" data-action="setup-open">Set up devd</button>')) +
      '<div class="ui-more">Also: <button type="button" data-action="nav" data-view="toolchain">Tools</button> \u00b7 <button type="button" data-action="nav" data-view="runtimes">Runtimes</button> \u00b7 <button type="button" data-action="nav" data-view="databases">Databases</button> \u00b7 <button type="button" data-action="nav" data-view="remote">Remote</button> \u00b7 <button type="button" data-action="nav" data-view="system">Diagnostics</button></div>');
  }

  // --------------------------------------------------------------------- sites

  /* What a site is served as, said once, from what the server reports. The server decides
     (devd-core serving.rs); this only words it. A PHP version appears only for a site that
     serves PHP, and says whether it is pinned or follows the default. */
  function servingLabel(row) {
    switch (row.serving) {
      case "proxy": return "proxy" + (row.site && row.site.upstream ? " → " + row.site.upstream : "");
      case "php": return "PHP " + (row.php_effective || "") + (row.php_source === "inherited" ? " \u00b7 default" : "");
      case "static": return "static files";
      case "no_entry": return "no web entry point";
      default: return "";
    }
  }

  function siteNeedsAttention(row) {
    return row.present === false || row.serving === "no_entry" ||
      (row.site && row.site.tls_mode !== "off" && !row.certificate);
  }

  function siteAttentionReason(row) {
    if (row.present === false) return "the folder is missing";
    if (row.serving === "no_entry") return "nothing in the folder can be served";
    if (row.site && row.site.tls_mode !== "off" && !row.certificate) return "no HTTPS certificate";
    return "";
  }

  /* What the last site.verify said about a site, kept for this window only. A
     record, a certificate and a running web server are facts about configuration;
     this is the one fact about what a browser would get. */
  var SITE_VERIFY = {};

  function siteVerifyFact(name) {
    var v = SITE_VERIFY[name];
    if (!v) return "";
    if (v.pending) return '<span class="fact">' + uiDot("live") + "checking…</span>";
    var r = v.report || {};
    var when = v.at ? " at " + new Date(v.at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }) : "";
    // A 4xx is an answer from the right server, not the page a person wanted: say it plainly, without the green.
    if (r.ok) return '<span class="fact ' + (r.status >= 400 ? "fact-warn" : "fact-ok") + '" title="' + attr((r.detail || "") + when) + '">answers' + (r.status ? " HTTP " + r.status : "") + "</span>";
    return '<span class="fact fact-bad" title="' + attr((r.detail || v.error || "") + when) + '">' +
      esc(r.status ? "HTTP " + r.status : !r.resolves && r.host ? "does not resolve" : r.resolves && !r.listening ? "nothing listening" : "did not answer") + "</span>";
  }

  function renderSites() {
    if (state.view !== "sites") return;
    var all = (state.server.sites || []).slice().sort(function (a, b) {
      return String(a.site && a.site.name).localeCompare(String(b.site && b.site.name));
    });
    var q = state.filters.sites || "";
    var mode = state.siteFilter || "all";
    var noEntry = all.filter(function (row) { return row.serving === "no_entry"; });
    var serving = all.length - noEntry.length;
    var attention = all.filter(siteNeedsAttention);
    var pendingHosts = ((state.server.hosts && (state.server.hosts.adding || []).concat(state.server.hosts.removing || [])) || []).length;
    var inHosts = all.filter(function (row) { return row.in_hosts; }).length;
    var missingCerts = all.filter(function (row) { return row.site && row.site.tls_mode !== "off" && !row.certificate; }).length;
    var matching = all.filter(function (row) {
      if (mode === "serving" && row.serving === "no_entry") return false;
      if (mode === "attention" && !siteNeedsAttention(row)) return false;
      return keeps(q, row.site && row.site.name, row.url, row.document_root, row.php_effective, row.site && row.site.upstream, row.detected);
    });
    // One row per site, in columns that answer the questions in the order they are
    // asked: which address, what answers it, is it secure, did it actually respond.
    var rows = matching.map(function (row) {
      var site = row.site;
      var proxied = !!site.upstream;
      var reason = siteAttentionReason(row);
      var checked = SITE_VERIFY[site.name] && SITE_VERIFY[site.name].report;
      var pending = SITE_VERIFY[site.name] && SITE_VERIFY[site.name].pending;
      var dot = row.present === false ? "bad" : checked && !checked.ok ? "bad" : row.serving === "no_entry" || reason ? "warn" : row.server_running ? "ok" : "off";

      var target = proxied ? "to " + site.upstream : row.serving === "no_entry" ? "" : row.document_root || "";
      var serves = '<div class="st-main">' + esc(servingLabel(row) || "not serving") + "</div>" +
        (target ? '<div class="st-sub mono" title="' + attr(target) + '">' + esc(target) + "</div>" : "") +
        (proxied ? '<div class="st-sub ' + (row.upstream_listening ? "" : "st-bad") + '">' + (row.upstream_listening ? "upstream listening" : "upstream not running") + "</div>" : "") +
        (row.serving === "no_entry" ? '<div class="st-sub st-warn">' + esc(reason || "nothing to serve") + "</div>" : "");

      var tlsWord = site.tls_mode === "off" ? '<div class="st-main">HTTP only</div>'
        : row.certificate ? '<div class="st-main st-ok">Certificate</div>' : '<div class="st-main st-warn">No certificate</div>';
      var cert = tlsWord + '<div class="st-sub">' + (row.in_hosts ? "in hosts" : '<span class="st-warn">not in hosts</span>') + "</div>";

      var reach;
      if (pending) reach = '<div class="st-main">Checking…</div>';
      else if (checked) {
        var st = checked.status;
        reach = checked.ok
          ? '<div class="st-main ' + (st >= 400 ? "st-warn" : "st-ok") + '">Answered' + (st ? " " + st : "") + '</div><div class="st-sub">' + (st >= 400 ? "an answer, not the page" : "request succeeded") + "</div>"
          : '<div class="st-main st-bad">' + esc(st ? "HTTP " + st : !checked.resolves ? "Does not resolve" : checked.resolves && !checked.listening ? "Nothing listening" : "No answer") + '</div><div class="st-sub st-bad" title="' + attr(checked.detail || "") + '">' + esc(checked.detail || "") + "</div>";
      } else reach = '<div class="st-main st-dim">Not checked</div><div class="st-sub">a listening port is not a healthy site</div>';

      return '<div class="st-row" data-open="1" data-action="open-site" data-name="' + attr(site.name) + '">' +
        '<div class="st-host">' + uiDot(dot) + '<div class="st-hostbody"><div class="st-name mono">' + esc(site.host || site.name) + '</div><div class="st-sub mono" title="' + attr(row.url) + '">' + esc(row.url) +
          (row.shared ? '</div><div class="st-sub"><a href="' + attr(row.shared) + '" target="_blank" rel="noreferrer">public link</a>' : "") + "</div></div></div>" +
        '<div class="st-col" data-label="Serves">' + serves + "</div>" +
        '<div class="st-col" data-label="Security">' + cert + "</div>" +
        '<div class="st-col" data-label="Reachability">' + reach + "</div>" +
        '<div class="st-actions">' +
          '<button type="button" class="btn btn-mini" data-action="verify-site" data-name="' + attr(site.name) + '"' + (pending ? " disabled" : "") + ' title="Request the address and report what answered">Check</button>' +
          '<button type="button" class="btn btn-mini btn-ghost" data-action="open-url" data-url="' + attr(row.url) + '" aria-label="Open ' + attr(row.url) + ' in the browser">Open</button>' +
          '<button type="button" class="btn btn-mini" data-action="open-site" data-name="' + attr(site.name) + '" aria-label="Manage ' + attr(site.name) + '">Manage</button>' +
        "</div></div>";
    });
    var table = '<div class="st-table" role="table" aria-label="Sites">' +
      '<div class="st-headrow" role="row"><span>Address</span><span>Serves</span><span>Security</span><span>Reachability</span><span></span></div>' + rows.join("") + "</div>";
    var filters = '<div class="segmented site-filter" role="group" aria-label="Filter sites">' +
      '<button type="button" class="seg' + (mode === "all" ? " active" : "") + '" data-action="site-filter" data-filter="all">All <span class="seg-count">' + all.length + "</span></button>" +
      '<button type="button" class="seg' + (mode === "serving" ? " active" : "") + '" data-action="site-filter" data-filter="serving">Serving <span class="seg-count">' + serving + "</span></button>" +
      '<button type="button" class="seg' + (mode === "attention" ? " active" : "") + '" data-action="site-filter" data-filter="attention">Needs attention <span class="seg-count">' + attention.length + "</span></button></div>";
    var body = state.siteFormOpen ? siteForm() : "";
    if (all.length) {
      body += summaryBand([
        { label: "Sites", value: all.length + (all.length === 1 ? " site" : " sites"), detail: serving + " can serve · " + noEntry.length + " with no entry point" },
        { label: "Needs attention", value: attention.length ? String(attention.length) : "None", detail: attention.length ? (missingCerts ? missingCerts + " missing a certificate" : "open a site to see what to fix") : "every site can serve", tone: attention.length ? "warn" : "ok", dot: attention.length ? "warn" : "ok" },
        { label: "Hostnames", value: pendingHosts ? pendingHosts + " pending" : "Published", detail: inHosts + " of " + all.length + " resolve locally", tone: pendingHosts ? "warn" : "", dot: pendingHosts ? "warn" : "off" },
        { label: "Local HTTPS", value: caTrusted() ? "Trusted" : "Not trusted", detail: (state.server.certs || []).length + " certificates issued", tone: caTrusted() ? "ok" : "warn", dot: caTrusted() ? "ok" : "warn" }
      ]);
      body += '<div class="ui-controls site-controls">' +
        '<input type="search" class="ui-search" data-filter="sites" placeholder="Search by name, address, folder or runtime" value="' + attr(q) + '" aria-label="Search sites">' +
        '<span class="grow"></span>' + filters + "</div>";
      body += matching.length ? table : uiEmpty('No site matches "' + q + '"', "Clear the search or choose another filter.", '<button type="button" class="btn" data-action="site-filter" data-filter="all">Show all sites</button>');
    } else if (serverFailed("site.list")) {
      body += uiEmpty("Couldn't read your sites", serverFailed("site.list") + ". The list is not empty; devd did not answer the request.", '<button type="button" class="btn" data-action="refresh">Try again</button>');
    } else if (!state.siteFormOpen) {
      body += uiEmpty("No sites yet", "Give a project folder an address and devd handles the hostname, the certificate and the HTTPS.", '<button type="button" class="btn btn-primary" data-action="new-site">Create a site</button>');
    }
    if (all.length) body += '<div class="ui-more"><b>Verify</b> requests each address the way a browser would: the hostname, the listener, the TLS handshake and the HTTP status. A certificate on disk or a running web server is not proof that a site answers.</div>';
    el("main").innerHTML = page("Sites", "",
      (all.length ? '<button type="button" class="btn" data-action="verify-sites">Check all</button>' : "") +
      '<button type="button" class="btn btn-primary" data-action="new-site">New site</button>', body);
  }

  // --------------------------------------------------------------- operations
  //
  // What a server panel shows for a site, for the machine you develop on: the .env, the
  // scheduler, queue workers, redirects, logs, deploys, alerts, PHP and Ploi. Every control
  // calls a capability, so an agent can do the same thing and the same approvals apply.

  var OPS = { site: "", tab: "env", data: {}, loading: false, error: "", errorTab: "" };
  var OPS_LOADING = {};
  var OPS_TABS = [["env", "Environment"], ["cron", "Scheduler"], ["queue", "Workers"], ["redirects", "Redirects"], ["logs", "Logs"], ["laravel", "Laravel"], ["nginx", "Nginx"], ["deploy", "Deploy"], ["db", "Databases"], ["health", "Health"], ["servers", "Servers"], ["keys", "SSH keys"], ["notify", "Alerts"], ["php", "PHP"], ["ploi", "Ploi"]];
  var OPS_SITELESS = { overview: 1, notify: 1, php: 1, ploi: 1, db: 1, health: 1, servers: 1, keys: 1 };
  var SITE = { name: "", tab: "overview" };
  var SITE_TABS = [["overview", "Overview"], ["env", "Environment"], ["deploy", "Deploy"], ["cron", "Cron"], ["queue", "Queue"], ["logs", "Logs"], ["laravel", "Laravel"], ["redirects", "Redirects"], ["nginx", "Web server"], ["db", "Database"], ["preview", "Preview"]];
  var REMOTE = { tab: "servers" };
  var REMOTE_TABS = [["servers", "Servers"], ["keys", "SSH keys"], ["ploi", "Ploi"]];
  var NOTIFY = { data: null, loading: false, error: "" };
  var MACHINE = { data: null, loading: false, error: "" };

  function opsSiteNames() {
    return (state.server.sites || []).map(function (r) { return r.site && r.site.name; }).filter(Boolean);
  }

  function opsEnsureSite() {
    var names = opsSiteNames();
    if (!OPS.site || names.indexOf(OPS.site) < 0) OPS.site = names[0] || "";
  }

  function siteRowByName(name) {
    return (state.server.sites || []).filter(function (r) { return r.site && r.site.name === name; })[0] || null;
  }

  function verifySite(name, quiet) {
    SITE_VERIFY[name] = { pending: true };
    render();
    return run("site.verify", { name: name }, { quiet: true }).then(function (o) {
      SITE_VERIFY[name] = o && o.data ? { report: o.data, at: Date.now() } : { report: { ok: false, detail: (o && o.message) || "the check could not run" }, at: Date.now() };
      if (!quiet) toast(SITE_VERIFY[name].report.ok ? "ok" : "warn", SITE_VERIFY[name].report.detail || name);
      render();
    });
  }

  function openSite(name) {
    if (!name) return;
    SITE.name = name;
    SITE.tab = "overview";
    OPS.site = name;
    OPS.data = {};
    OPS.error = "";
    OPS.errorTab = "";
    navigate("site");
  }

  function activeOpsTab() {
    if (state.view === "site") return SITE.tab === "preview" ? "overview" : SITE.tab;
    if (state.view === "databases") return "db";
    if (state.view === "remote") return REMOTE.tab;
    if (state.view === "runtimes") return "php";
    return OPS.tab;
  }

  function opsLoad(tab) {
    tab = tab || OPS.tab;
    opsEnsureSite();
    var key = tab + "|" + (OPS_SITELESS[tab] ? "" : OPS.site);
    if (OPS_LOADING[key]) return Promise.resolve();
    OPS_LOADING[key] = true;
    OPS.loading = true;
    OPS.error = "";
    OPS.errorTab = "";
    var s = { site: OPS.site };
    var jobs;
    if (!OPS_SITELESS[tab] && !OPS.site) {
      OPS_LOADING[key] = false;
      OPS.loading = false;
      render();
      return Promise.resolve();
    }
    if (tab === "overview") jobs = [read("php.settings", {})];
    else if (tab === "env") jobs = [read("env.get", s)];
    else if (tab === "cron") jobs = [read("cron.list", s)];
    else if (tab === "queue") jobs = [read("queue.list", s)];
    else if (tab === "redirects") jobs = [read("redirects.list", s)];
    else if (tab === "logs") jobs = [read("logs.laravel", { site: OPS.site, limit: 40, levels: OPS.levels || [] }), read("logs.site", { site: OPS.site, kind: OPS.logKind || "nginx_error", lines: 80 })];
    else if (tab === "deploy") jobs = [read("deploy.history", { site: OPS.site, limit: 15 }), read("deploy.script", s), read("repo.status", s), read("deploy.check", s), read("remote.links", s), read("deploy.vars", s), read("deploy.scheduled", s), read("deploy.staging", {})];
    else if (tab === "laravel") jobs = [read("laravel.status", s)];
    else if (tab === "nginx") jobs = [read("nginx.site_show", s), read("nginx.presets", {})];
    else if (tab === "db") jobs = [read("db.users", {}), read("backup.list", {})];
    else if (tab === "health") jobs = [read("health.list", {}), read("monitor.stats", {})];
    else if (tab === "servers") jobs = [read("server.list", {}), read("remote.links", {})];
    else if (tab === "keys") jobs = [read("ssh.keys", {})];
    else if (tab === "notify") jobs = [read("notify.list", {})];
    else if (tab === "php") jobs = [read("php.settings", {}), read("php.extensions", {})];
    else jobs = [read("ploi.status", {})];
    return Promise.all(jobs).then(function (res) {
      if (!OPS_SITELESS[tab] && OPS.site !== s.site) return null;
      OPS.data[tab] = res;
      if (tab === "ploi" && res[0] && res[0].connected) {
        return read("ploi.servers", {}).then(function (srv) { OPS.data.ploiServers = srv && srv.servers || []; });
      }
      return null;
    }).catch(function (e) {
      if (!OPS_SITELESS[tab] && OPS.site !== s.site) return;
      OPS.error = e.message;
      OPS.errorTab = tab;
    }).then(function () {
      OPS_LOADING[key] = false;
      OPS.loading = false;
      if (state.view === "operations" || state.view === "site" || state.view === "databases" || state.view === "remote" || state.view === "runtimes") render();
    });
  }

  function opsAct(id, args, after) {
    return run(id, args).then(function (o) {
      if (o && after) after(o);
      return o ? opsLoad(activeOpsTab()) : null;
    });
  }

  function notifyAct(id, args) {
    return run(id, args).then(function (o) {
      if (!o) return null;
      NOTIFY.data = null;
      NOTIFY.error = "";
      if (state.view === "system") return loadNotifications();
      return opsLoad("notify");
    });
  }

  function opsTabs() {
    return '<div class="segmented ops-tabs">' + OPS_TABS.map(function (t) {
      return '<button type="button" class="seg' + (OPS.tab === t[0] ? " active" : "") + '" data-action="ops-tab" data-tab="' + t[0] + '">' + esc(t[1]) + "</button>";
    }).join("") + "</div>";
  }

  function opsSitePicker() {
    var names = opsSiteNames();
    if (!names.length || OPS_SITELESS[OPS.tab]) return "";
    return '<div class="ops-site"><label for="ops-site">Site</label><select id="ops-site" class="select compact" data-change="ops-site">' +
      names.map(function (n) { return '<option value="' + attr(n) + '"' + (n === OPS.site ? " selected" : "") + ">" + esc(n) + "</option>"; }).join("") + "</select></div>";
  }

  function opsBody(tab) {
    tab = tab || OPS.tab;
    if (OPS.error && OPS.errorTab === tab) return '<div class="notice notice-bad">' + esc(OPS.error) + "</div>";
    var d = OPS.data[tab];
    if (!d) return '<div class="loading"></div>';
    var fn = { env: opsEnv, cron: opsCron, queue: opsQueue, redirects: opsRedirects, logs: opsLogs, laravel: opsLaravel, nginx: opsNginx, deploy: opsDeploy, db: opsDb, health: opsHealth, servers: opsServers, keys: opsKeys, notify: opsNotify, php: opsPhp, ploi: opsPloi }[tab];
    return fn ? fn(d) : "";
  }

  function renderOperations() {
    if (state.view !== "operations") return;
    opsEnsureSite();
    var needSite = !OPS_SITELESS[OPS.tab] && !OPS.site;
    var body = opsTabs() + opsSitePicker() +
      (needSite ? uiEmpty("No sites yet", "Create a site first; operations belong to a site.", '<button type="button" class="btn btn-primary" data-action="nav" data-view="sites">Create a site</button>') : opsBody(OPS.tab));
    el("main").innerHTML = page("Operations", "", "", body);
    if (!OPS.data[OPS.tab] && !needSite && !(OPS.error && OPS.errorTab === OPS.tab)) opsLoad(OPS.tab);
  }

  // --------------------------------------------------------------- site detail
  //
  // A site has enough of its own settings to deserve a page. The site comes
  // from the address, not from a picker, so every control on the page acts on
  // the site named at the top. The old Operations screen stays reachable as a
  // fallback for one release.

  function siteTabs() {
    return '<div class="segmented site-tabs">' + SITE_TABS.map(function (t) {
      return '<button type="button" class="seg' + (SITE.tab === t[0] ? " active" : "") + '" data-action="site-tab" data-tab="' + t[0] + '">' + esc(t[1]) + "</button>";
    }).join("") + "</div>";
  }

  /* The checks site.verify makes, in the order they can fail. Nothing is shown as
     passed until a check has run; each step names what was observed. */
  function siteReachability(site) {
    var v = SITE_VERIFY[site.name];
    var r = v && v.report;
    var button = '<button type="button" class="btn btn-mini' + (r ? "" : " btn-primary") + '" data-action="verify-site" data-name="' + attr(site.name) + '"' + (v && v.pending ? " disabled" : "") + ">" +
      (v && v.pending ? "Checking…" : r ? "Check again" : "Check now") + "</button>";
    if (!r) {
      return uiHead("Reachability", null, button) +
        uiEmpty(v && v.pending ? "Requesting the address…" : "Not checked in this window",
          "Checks the hostname, that it resolves, that something listens on the port, the TLS handshake and the HTTP status - the way a browser would reach it. One request; it changes nothing.");
    }
    var tls = /^https:/.test(r.url || "");
    function step(label, ok, yes, no, skipped) {
      var kind = skipped ? "off" : ok ? "ok" : "bad";
      return uiRow({ dot: kind, title: esc(label) + ' <span class="facts">' +
        (skipped ? '<span class="fact">not reached</span>' : ok ? '<span class="fact fact-ok">' + esc(yes) + "</span>" : '<span class="fact fact-bad">' + esc(no) + "</span>") + "</span>" });
    }
    var steps = [
      step("Hostname", r.in_hosts || r.resolves, r.in_hosts ? "in devd's hosts block" : "resolves without it", "not in hosts", false),
      step("Resolves", r.resolves, "resolves", "does not resolve", false),
      step("Listener", r.listening, "something listens", "nothing listening", !r.resolves),
      tls ? step("TLS", r.tls, "handshake chains to devd's authority", "handshake failed", !r.listening) : "",
      step("HTTP", r.ok, "answered " + (r.status || "") + (r.status >= 400 ? " - an answer, but not the page" : ""), r.status ? "answered " + r.status : "no answer", !r.listening || (tls && !r.tls))
    ].filter(Boolean);
    return uiHead("Reachability", null, '<span class="ui-quiet" style="padding:0">checked ' + esc(new Date(v.at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })) + "</span>" + button) +
      (r.ok ? "" : '<div class="notice notice-bad" role="alert"><b>' + esc(r.host || site.name) + " is not reachable.</b> " + esc(r.detail || "") + "</div>") +
      uiList(steps);
  }

  function siteOverview(row) {
    var site = row.site;
    var runtime = row.serving === "proxy" ? "proxy to " + site.upstream
      : row.serving === "php" ? "PHP " + (row.php_effective || "") + (row.php_source === "pinned" ? " (pinned for this site)" : " (follows the default)")
      : row.serving === "static" ? "static files"
      : "not served: " + (row.detected || "nothing to serve");
    var profileData = OPS.data.overview && OPS.data.overview[0];
    var own = profileData ? ((profileData.site_profiles || []).filter(function (p) { return p.site === site.name; })[0] || {}).profile || "" : "";
    var installed = row.php_installed || [];
    var pinned = site.php ? devdSeries(site.php) : "";
    var phpSelect = row.serving === "php" && installed.length
      ? '<select class="select compact" data-action="site-php" data-name="' + attr(site.name) + '" aria-label="PHP version for ' + attr(site.name) + '"><option value=""' + (pinned ? "" : " selected") + ">Default PHP</option>" +
        installed.map(function (v) { return '<option value="' + attr(v) + '"' + (pinned === v ? " selected" : "") + ">" + esc(v) + "</option>"; }).join("") + "</select>"
      : "";
    var profileSelect = profileData
      ? '<select class="select compact" data-change="ops-site-profile" data-site="' + attr(site.name) + '" aria-label="PHP limits for ' + attr(site.name) + '"><option value=""' + (own ? "" : " selected") + '>Follow the machine (' + esc(profileData.profile || "local") + ")</option>" +
        '<option value="local"' + (own === "local" ? " selected" : "") + '>Local limits</option><option value="production"' + (own === "production" ? " selected" : "") + ">Production limits</option></select>"
      : "";
    var details = [
      uiRow({
        dot: row.server_running ? "ok" : "off",
        title: esc(row.url),
        sub: esc(row.document_root || "") + (row.present === false ? " (directory missing)" : ""),
        actions: '<button type="button" class="btn btn-mini btn-primary" data-action="open-url" data-url="' + attr(row.url) + '">Open</button>' +
          '<button type="button" class="btn btn-mini btn-ghost" data-action="site-tab" data-tab="preview">Preview</button>'
      }),
      uiRow({
        title: "Project",
        sub: '<span class="mono">' + esc(row.project || site.root) + "</span>" + (row.project_known ? "" : " \u00b7 not registered in Projects yet"),
        actions: '<button type="button" class="btn btn-mini btn-ghost" data-action="home-open" data-path="' + attr(row.project) + '">' + (row.project_known ? "Open project" : "Open folder") + "</button>"
      }),
      uiRow({ title: "Serves", sub: esc(runtime) + (row.upstream_listening === false ? " (not running)" : "") + (row.serving_source === "detected" && row.serving !== "proxy" ? " \u00b7 read from the folder" : row.serving_source === "configured" ? " \u00b7 your choice" : "") }),
      uiRow({ title: "TLS", sub: site.tls_mode === "off" ? "off" : row.certificate ? "HTTPS, certificate present" : "HTTPS, no certificate yet" }),
      uiRow({ title: "Database", sub: site.db_name ? '<span class="mono">' + esc(site.db_name) + "</span>" : "not managed by devd", actions: '<button type="button" class="btn btn-mini btn-ghost" data-action="site-tab" data-tab="db">Database</button>' }),
      uiRow({ title: "Sharing", sub: row.shared ? '<a href="' + attr(row.shared) + '" target="_blank" rel="noreferrer">' + esc(row.shared) + "</a>" : "only on this machine" })
    ];
    var servingSelect = site.upstream ? "" :
      '<select class="select compact" data-change="site-serving" data-name="' + attr(site.name) + '" aria-label="What ' + attr(site.name) + ' serves">' +
      '<option value=""' + (site.serving ? "" : " selected") + ">Read from the folder</option>" +
      '<option value="php"' + (site.serving === "php" ? " selected" : "") + ">PHP</option>" +
      '<option value="static"' + (site.serving === "static" ? " selected" : "") + ">Static files</option></select>";
    var rowsSettings = [];
    if (servingSelect) rowsSettings.push(uiRow({ title: "What it serves", sub: site.serving ? "set to " + esc(site.serving) : "read from the folder: " + esc(row.detected || ""), actions: servingSelect }));
    if (row.serving === "php") {
      rowsSettings.push(uiRow({ title: "PHP version", sub: pinned ? "pinned to " + esc(pinned) : "follows the machine default", actions: phpSelect }));
      rowsSettings.push(uiRow({ title: "PHP limits", sub: own ? "uses the " + esc(own) + " profile" : "follows the machine", actions: profileSelect + '<button type="button" class="btn btn-mini btn-ghost" data-action="nav" data-view="runtimes">Runtimes</button>' }));
    } else if (row.php_unused_pin) {
      rowsSettings.push(uiRow({ title: "PHP " + esc(row.php_unused_pin) + " is recorded but unused", sub: "this site does not serve PHP, so the version does nothing", actions: '<button type="button" class="btn btn-mini btn-ghost" data-action="site-clear-php" data-name="' + attr(site.name) + '">Clear it</button>' }));
    }
    var settings = uiList(rowsSettings);
    if (!OPS.data.overview && !(OPS.error && OPS.errorTab === "overview")) opsLoad("overview");
    var loadError = OPS.error && OPS.errorTab === "overview" ? '<div class="notice notice-bad">' + esc(OPS.error) + "</div>" : "";
    return siteReachability(site) + uiHead("Site", null, "") + uiList(details) +
      uiHead("Serving", null, "") + settings + loadError +
      '<div class="ui-more">Environment, deploys, cron, workers, logs, Laravel, redirects and the web server are on the tabs above.</div>';
  }

  function siteDatabase(row) {
    var db = row.site.db_name;
    var open = '<button type="button" class="btn btn-mini btn-ghost" data-action="nav" data-view="databases">All databases</button>';
    if (!db) {
      return uiHead("Database", null, "") +
        uiEmpty("No database managed by devd", "This site was created without one. If the application uses SQLite or a database on another server, it is not managed here. Database users and backups are under Environment \u2192 Databases.", '<button type="button" class="btn btn-primary" data-action="nav" data-view="databases">Open Databases</button>');
    }
    var data = OPS.data.db;
    var backups = data ? ((data[1] && data[1].schedules) || []).filter(function (b) { return String(b.database) === String(db); }) : [];
    var backupNote = !data
      ? (OPS.error && OPS.errorTab === "db" ? "backup schedules could not be read" : "checking backup schedules")
      : backups.length
        ? backups.length + (backups.length === 1 ? " scheduled backup" : " scheduled backups")
        : "no scheduled backups";
    return uiHead("Database", null, open) +
      uiList([uiRow({ title: '<span class="mono">' + esc(db) + "</span>", sub: "this site's database on the devd server \u00b7 " + esc(backupNote) })]) +
      '<p class="dim ops-note">Database users and backups belong to the whole environment. Manage users, schedules and dump files under Environment \u2192 Databases.</p>';
  }

  function sitePreview(row) {
    var notice = caTrusted()
      ? ""
      : '<div class="notice" style="margin-bottom:12px"><div class="grow">The certificate authority is not trusted yet, so an HTTPS preview may show a warning.</div>' +
        '<button type="button" class="btn btn-mini" data-action="ca-trust" data-machine="false">Trust the CA</button></div>';
    return notice +
      '<div class="panel"><div class="panel-head"><span class="mono">' + esc(row.url) + '</span><span class="panel-note">' +
      '<button type="button" class="btn btn-mini btn-ghost" data-action="reload-preview">Reload</button>' +
      '<button type="button" class="btn btn-mini btn-ghost" data-action="open-url" data-url="' + attr(row.url) + '">Open externally</button>' +
      "</span></div>" +
      '<div class="panel-body flush"><iframe id="preview-frame" class="preview-frame" sandbox="allow-scripts allow-same-origin allow-forms allow-popups allow-downloads allow-modals" src="' +
      attr(row.url) + '" title="' + attr(row.site.name) + '"></iframe></div></div>';
  }

  function siteBody(row) {
    var tab = SITE.tab;
    if (tab === "overview") return siteOverview(row);
    if (tab === "preview") return sitePreview(row);
    if (tab === "db") {
      if (!OPS.data.db && !(OPS.error && OPS.errorTab === "db")) opsLoad("db");
      return siteDatabase(row);
    }
    if (!OPS.data[tab] && !(OPS.error && OPS.errorTab === tab)) opsLoad(tab);
    return opsBody(tab);
  }

  function renderSite() {
    if (state.view !== "site") return;
    var row = siteRowByName(SITE.name);
    if (!row) {
      el("main").innerHTML = page("Site", "", '<button type="button" class="btn" data-action="nav" data-view="sites">All sites</button>',
        uiEmpty("Site not found", "It may have been removed, or the address may be old.", '<button type="button" class="btn btn-primary" data-action="nav" data-view="sites">Back to sites</button>'));
      return;
    }
    SITE.name = row.site.name;
    OPS.site = SITE.name;
    var actions = '<button type="button" class="btn" data-action="open-url" data-url="' + attr(row.url) + '">Open</button>' +
      (row.shared
        ? '<button type="button" class="btn" data-action="unshare-site" data-name="' + attr(SITE.name) + '">Stop sharing</button>'
        : '<button type="button" class="btn" data-action="share-site" data-name="' + attr(SITE.name) + '">Share</button>') +
      '<button type="button" class="btn btn-ghost" data-action="remove-site" data-name="' + attr(SITE.name) + '">Remove</button>';
    el("main").innerHTML = page(SITE.name, "", actions, siteTabs() + siteBody(row));
  }

  // ---------------------------------------------------------------- databases

  function renderDatabases() {
    if (state.view !== "databases") return;
    var failed = OPS.error && OPS.errorTab === "db";
    var mariadb = (state.server.services || []).filter(function (s) { return s.name === "mariadb"; })[0];
    var stopped = mariadb && !mariadb.running;
    if (!OPS.data.db && !failed && !stopped) opsLoad("db");
    var body;
    if (stopped) {
      var mdb = (state.server.services || []).filter(function (x) { return x.name === "mariadb"; })[0];
      body = mdb && mdb.listening
        ? '<div class="notice notice-warn" role="status"><b>Port ' + esc(mdb.port) + " is held by a database devd did not start.</b> " +
          esc(mdb.ready_detail ? "It " + mdb.ready_detail + ". " : "") + "devd's own MariaDB is stopped, and starting it here would collide with that process. Stop the other one first, or give devd's MariaDB another port.</div>" +
          uiEmpty("devd's MariaDB is not running", "Users, grants and scheduled backups live inside the MariaDB devd runs. Nothing is changed by looking.", '<button type="button" class="btn" data-action="databases-start">Start MariaDB anyway</button>')
        : uiEmpty("MariaDB is not running", "Users, grants and scheduled backups live inside MariaDB. Start it to inspect them; nothing is changed by looking.", '<button type="button" class="btn btn-primary" data-action="databases-start">Start MariaDB</button>');
    } else if (failed) {
      body = '<div class="notice notice-bad">' + esc(OPS.error) + "</div>" +
        '<div class="inline"><button type="button" class="btn btn-primary" data-action="databases-start">Start MariaDB</button><button type="button" class="btn" data-action="nav" data-view="services">Open Services</button></div>';
    } else {
      body = OPS.data.db ? opsDb(OPS.data.db) : '<div class="loading" role="status" aria-live="polite">Reading databases…</div>';
    }
    el("main").innerHTML = page("Databases", "", "",
      '<div class="ui-quiet">Users and scheduled backups for the databases devd runs.</div>' + body);
  }

  // ------------------------------------------------------------------- remote

  function remoteTabs() {
    return '<div class="segmented remote-tabs">' + REMOTE_TABS.map(function (t) {
      return '<button type="button" class="seg' + (REMOTE.tab === t[0] ? " active" : "") + '" data-action="remote-tab" data-tab="' + t[0] + '">' + esc(t[1]) + "</button>";
    }).join("") + "</div>";
  }

  function renderRemote() {
    if (state.view !== "remote") return;
    var tab = REMOTE.tab;
    var failed = OPS.error && OPS.errorTab === tab;
    if (!OPS.data[tab] && !failed) opsLoad(tab);
    var body = failed
      ? '<div class="notice notice-bad">' + esc(OPS.error) + "</div>"
      : OPS.data[tab] ? opsBody(tab) : '<div class="loading"></div>';
    el("main").innerHTML = page("Remote", "", "", remoteTabs() +
      '<div class="ui-quiet">Servers you deploy to, the keys that reach them, and Ploi.</div>' + body);
  }

  // ------------------------------------------------------------- environment

  function opsEnv(d) {
    var e = d[0] || {};
    var vars = e.vars || {};
    var names = Object.keys(vars);
    var rows = names.map(function (k) {
      var hidden = /^\[hidden/.test(vars[k]);
      return uiRow({
        title: '<span class="mono">' + esc(k) + "</span>",
        sub: hidden ? '<span class="dim">' + esc(vars[k]) + "</span>" : '<span class="mono">' + esc(vars[k] === "" ? "(empty)" : vars[k]) + "</span>",
        wrap: true, hover: true,
        actions: '<button type="button" class="btn btn-mini btn-ghost" data-action="ops-env-set" data-name="' + attr(k) + '">Change</button>' +
          '<button type="button" class="btn btn-mini btn-ghost" data-action="ops-env-unset" data-name="' + attr(k) + '">Remove</button>'
      });
    });
    var actions = '<button type="button" class="btn btn-mini btn-primary" data-action="ops-env-set">Add variable</button>' +
      '<button type="button" class="btn btn-mini" data-action="ops-env-diff">Compare…</button>' +
      '<button type="button" class="btn btn-mini btn-ghost" data-action="ops-env-reveal">Show secrets</button>';
    return uiHead(".env", names.length, actions) +
      (e.exists ? "" : '<div class="notice notice-warn">This project has no .env file yet. Adding a variable creates one.</div>') +
      (names.length ? uiList(rows) : uiEmpty("No variables", "")) +
      '<p class="dim ops-note">Secrets are hidden. Every change keeps the old file as <code>.env.devd-backup-&lt;time&gt;</code>.</p>';
  }

  // --------------------------------------------------------------- scheduler

  /* A table that keeps its columns: the head names them, every row answers the
     same questions, and the actions sit in their own column at the end. */
  function opsTable(cols, rows, label) {
    return '<div class="table-wrap"><table class="table table-ops" aria-label="' + attr(label) + '"><thead><tr>' +
      cols.map(function (c) { return "<th" + (c.cls ? ' class="' + c.cls + '"' : "") + ">" + esc(c.name) + "</th>"; }).join("") +
      '<th class="cell-actions"><span class="sr-only">Actions</span></th></tr></thead><tbody>' + rows.join("") + "</tbody></table></div>";
  }
  function opsDot(kind) { return '<span class="ui-dot ui-dot-' + kind + '" aria-hidden="true"></span>'; }

  function opsCron(d) {
    var jobs = (d[0] && d[0].jobs) || [];
    var rows = jobs.map(function (j) {
      var ok = j.last_exit === null || j.last_exit === undefined ? "off" : j.last_exit === 0 ? "ok" : "bad";
      var last = j.last_run_at ? esc(shortTime(j.last_run_at)) + ' <span class="' + (j.last_exit === 0 ? "dim" : "cell-bad") + '">' + (j.last_exit === 0 ? "worked" : "exit " + esc(String(j.last_exit))) + "</span>" : '<span class="dim">hasn\'t run</span>';
      return "<tr" + (j.enabled ? "" : ' class="is-off"') + "><td>" + opsDot(j.enabled ? ok : "off") + '<span class="mono">' + esc(j.schedule) + "</span></td>" +
        '<td class="cell-mono">' + esc(j.command) + "</td>" +
        "<td>" + (j.enabled ? esc(j.next_run_at ? shortTime(j.next_run_at) : "never") : '<span class="dim">turned off</span>') + "</td>" +
        "<td>" + last + "</td>" +
        '<td class="cell-actions"><button type="button" class="btn btn-mini" data-action="ops-cron-run" data-id="' + attr(j.id) + '">Run now</button>' +
          '<button type="button" class="btn btn-mini btn-ghost" data-action="ops-cron-log" data-id="' + attr(j.id) + '" data-log="' + attr(j.log) + '">Log</button>' +
          '<button type="button" class="btn btn-mini btn-ghost" data-action="ops-cron-toggle" data-id="' + attr(j.id) + '" data-on="' + (j.enabled ? "0" : "1") + '">' + (j.enabled ? "Turn off" : "Turn on") + "</button>" +
          '<button type="button" class="btn btn-mini btn-ghost" data-action="ops-cron-remove" data-id="' + attr(j.id) + '">Remove</button></td></tr>';
    });
    var actions = '<button type="button" class="btn btn-mini btn-primary" data-action="ops-cron-add">Add job</button>' +
      '<button type="button" class="btn btn-mini" data-action="ops-cron-laravel">Run the Laravel scheduler</button>';
    return uiHead("Cron jobs", jobs.length, actions) +
      (jobs.length ? opsTable([{ name: "Schedule" }, { name: "Command" }, { name: "Next run" }, { name: "Last run" }], rows, "Cron jobs") : uiEmpty("Nothing scheduled yet", "A cron job runs a command on a schedule, in this site's folder, with this site's PHP. A Laravel site needs just one: its scheduler, every minute.")) +
      '<p class="dim ops-note">Jobs only run while devd is open. If one comes due while devd is closed, it is skipped, not run late.</p>';
  }

  // ----------------------------------------------------------------- workers

  function opsQueue(d) {
    var list = (d[0] && d[0].workers) || [];
    var rows = list.map(function (w) {
      var dot = w.running === w.processes ? "ok" : w.running ? "warn" : "off";
      return "<tr><td>" + opsDot(dot) + "<b>" + esc(w.name) + "</b></td>" +
        '<td class="cell-mono">' + esc(w.command) + "</td>" +
        "<td>" + (w.running === w.processes ? "all " : "") + w.running + " of " + w.processes + " running</td>" +
        '<td class="cell-actions"><button type="button" class="btn btn-mini" data-action="ops-queue" data-do="' + (w.running ? "restart" : "start") + '" data-id="' + attr(w.id) + '">' + (w.running ? "Restart" : "Start") + "</button>" +
          (w.running ? '<button type="button" class="btn btn-mini btn-ghost" data-action="ops-queue" data-do="stop" data-id="' + attr(w.id) + '">Stop</button>' : "") +
          '<button type="button" class="btn btn-mini btn-ghost" data-action="ops-queue-log" data-id="' + attr(w.id) + '">Log</button>' +
          '<button type="button" class="btn btn-mini btn-ghost" data-action="ops-queue-remove" data-id="' + attr(w.id) + '">Remove</button></td></tr>';
    });
    var actions = '<button type="button" class="btn btn-mini btn-primary" data-action="ops-queue-add">Add queue worker</button>' +
      '<button type="button" class="btn btn-mini" data-action="ops-queue-add-command">Add a program</button>';
    return uiHead("Workers", list.length, actions) +
      (list.length ? opsTable([{ name: "Worker" }, { name: "Command" }, { name: "State" }], rows, "Queue workers") : uiEmpty("No workers running", "A worker keeps your queue moving: it runs `php artisan queue:work` with this site's PHP, and devd starts it again if it stops.")) +
      '<p class="dim ops-note">After a deploy, restart the worker so it picks up the new code.</p>';
  }

  // --------------------------------------------------------------- redirects

  function opsRedirects(d) {
    var list = (d[0] && d[0].redirects) || [];
    var rows = list.map(function (r) {
      var perm = r.status === 301 || r.status === 308;
      return '<tr><td class="cell-mono">' + esc(r.from) + '</td><td class="cell-mono">' + esc(r.to) + "</td>" +
        "<td>" + (perm ? "Permanent" : "Temporary") + ' <span class="dim">' + esc(String(r.status)) + "</span></td>" +
        '<td class="cell-actions"><button type="button" class="btn btn-mini btn-ghost" data-action="ops-redirect-remove" data-id="' + attr(r.id) + '">Remove</button></td></tr>';
    });
    return uiHead("Redirects", list.length, '<button type="button" class="btn btn-mini btn-primary" data-action="ops-redirect-add">Add redirect</button>') +
      (list.length ? opsTable([{ name: "From" }, { name: "To" }, { name: "Type" }], rows, "Redirects") : uiEmpty("No redirects yet", "Point a path at another path or an https address. `/blog/*` covers everything under /blog/.")) +
      '<p class="dim ops-note">Redirects are written into the web server\'s configuration, which is reloaded.</p>';
  }

  // -------------------------------------------------------------------- logs

  function opsLogs(d) {
    var lv = d[0] || {}, site = d[1] || {};
    var levels = ["ERROR", "WARNING", "INFO", "DEBUG"];
    var active = OPS.levels || [];
    var chips = '<div class="segmented">' + levels.map(function (l) {
      return '<button type="button" class="seg' + (active.indexOf(l) >= 0 ? " active" : "") + '" data-action="ops-level" data-level="' + l + '">' + l + "</button>";
    }).join("") + "</div>";
    var entries = (lv.entries || []).map(function (e) {
      var kind = /ERROR|CRITICAL|ALERT|EMERGENCY/.test(e.level) ? "bad" : e.level === "WARNING" ? "warn" : "off";
      return uiRow({ dot: kind, title: esc(e.message), sub: esc(e.at) + " · " + esc(e.env + "." + e.level) + (e.origin ? " · " + esc(e.origin) : ""), wrap: true });
    });
    var kinds = [["nginx_error", "Web server errors"], ["nginx_access", "Web server access"], ["php", "PHP"]];
    var picker = '<select class="select compact" data-change="ops-logkind">' + kinds.map(function (k) { return '<option value="' + k[0] + '"' + ((OPS.logKind || "nginx_error") === k[0] ? " selected" : "") + ">" + k[1] + "</option>"; }).join("") + "</select>";
    return uiHead("Laravel log", lv.matching, chips + '<button type="button" class="btn btn-mini btn-ghost" data-action="ops-log-clear">Empty</button>') +
      (lv.file ? (entries.length ? uiList(entries) : uiEmpty("Nothing matches", "No log entry fits that filter.")) : uiEmpty("No Laravel log yet", lv.note || "Once the app writes something, it shows up here.")) +
      uiHead("Server logs", null, picker) +
      '<pre class="log-block">' + esc((site.lines || []).join("\n") || (site.exists ? "The log is empty." : "No log yet: the site has not been requested.")) + "</pre>";
  }

  // ------------------------------------------------------------------ deploy

  function opsDeploy(d) {
    var hist = (d[0] && d[0].deployments) || [], script = d[1] || {}, repo = d[2] || {}, check = d[3] || {};
    // Four near-identical warnings are one finding: this machine is more generous than the
    // server. Each limit is a row (what it is, here, there, what that would do to a request),
    // and the way to settle it is stated once.
    var drift = (check.warnings || []).map(function (w) {
      var m = String(w).match(/^(\w+) is (.+?) here but (.+?) on (.+?)(?: \([^)]*\))?: (.+)$/);
      return m ? { name: m[1], here: m[2], there: m[3], where: m[4], effect: m[5] } : { raw: String(w) };
    });
    var warn = "";
    if (drift.length) {
      var where = (drift.filter(function (d) { return d.where; })[0] || {}).where || "your server";
      warn = '<section class="drift" aria-label="Differences from your server">' +
        "<h3>This machine allows more than " + esc(where) + " probably does</h3>" +
        '<p class="drift-lead">A site that works here can fail once it is deployed. ' + (check.server_assumed ? "The server's values below are assumed, not read from it." : "These are the limits you gave devd for the server.") + "</p>" +
        '<table class="drift-table"><thead><tr><th>Limit</th><th>Here</th><th>Server</th><th>What it would do</th></tr></thead><tbody>' +
        drift.map(function (d) {
          return d.raw ? '<tr><td colspan="4">' + esc(d.raw) + "</td></tr>"
            : '<tr><td class="mono">' + esc(d.name) + '</td><td class="mono">' + esc(d.here) + '</td><td class="mono">' + esc(d.there) + "</td><td>" + esc(d.effect.charAt(0).toUpperCase() + d.effect.slice(1)) + ".</td></tr>";
        }).join("") + "</tbody></table>" +
        '<div class="inline"><button type="button" class="btn btn-mini" data-action="ops-limits">' + (check.server_assumed ? "Tell devd what your server allows" : "Change the server limits") + '</button><button type="button" class="btn btn-mini btn-ghost" data-action="ops-site-profile-go" data-profile="production">Try this site with the server\'s limits</button></div></section>';
    }
    var rows = hist.map(function (x) {
      var ok = x.state === "success" ? "ok" : x.state === "failed" ? "bad" : "warn";
      return uiRow({
        dot: ok, title: esc(x.message || (x.state === "running" ? "Deploying…" : x.state)) + ' <span class="pill">' + esc(x.trigger) + "</span>",
        sub: esc(shortTime(x.started_at)) + (x.seconds ? " · " + Math.round(x.seconds) + "s" : "") + (x.commit_after ? " · " + esc(x.commit_after.slice(0, 8)) : "") + (x.failed_line ? " · failed at " + esc(x.failed_line) : ""),
        wrap: true, hover: true,
        actions: '<button type="button" class="btn btn-mini btn-ghost" data-action="ops-deploy-log" data-id="' + attr(x.id) + '">Output</button>' +
          (x.state === "success" && x.commit_after ? '<button type="button" class="btn btn-mini btn-ghost" data-action="ops-rollback" data-id="' + attr(x.id) + '">Roll back to this</button>' : "")
      });
    });
    var link = repo.linked && repo.link ? "Follows " + esc(repo.link.branch) + (repo.link.deploy_on_push ? " · push-to-deploy on" : "") : repo.is_repository ? "Not linked to a branch yet" : "Not a Git repository";
    var repoRow = uiRow({
      dot: repo.linked ? "ok" : "off", title: "Repository",
      sub: link + (repo.head ? " · at " + esc(String(repo.head).slice(0, 8)) : "") + (repo.uncommitted_files ? " · " + repo.uncommitted_files + " uncommitted" : ""),
      actions: (repo.is_repository ? '<button type="button" class="btn btn-mini" data-action="ops-repo-link">' + (repo.linked ? "Change" : "Link") + "</button>" : "") +
        (repo.linked ? '<button type="button" class="btn btn-mini btn-ghost" data-action="ops-hook" data-on="' + (repo.link && repo.link.deploy_on_push ? "0" : "1") + '">' + (repo.link && repo.link.deploy_on_push ? "Turn push-to-deploy off" : "Turn push-to-deploy on") + "</button>" : "")
    });
    var head = uiHead("Deployments", hist.length, '<button type="button" class="btn btn-mini btn-ghost" data-action="ops-script">' + (script.saved ? "Edit script" : "Deploy script") + '</button><button type="button" class="btn btn-mini btn-primary" data-action="ops-deploy">Deploy now</button>');
    return warn + uiList([repoRow]) + opsDeployExtras(d) + head + (hist.length ? uiList(rows) : uiEmpty("Nothing deployed yet", "A deploy runs this script: " + (script.saved ? "yours." : "the default one for this project. Open it to see or change it.")));
  }

  // ------------------------------------------------------------------ alerts

  function opsNotify(d) {
    var list = (d[0] && d[0].targets) || [];
    var rows = list.map(function (t) {
      return uiRow({
        title: esc(t.channel) + (t.site ? ' <span class="pill">' + esc(t.site) + "</span>" : ""),
        sub: esc(t.destination) + " · " + esc(t.events.join(", ")), wrap: true, hover: true,
        actions: '<button type="button" class="btn btn-mini" data-action="ops-notify-test" data-id="' + attr(t.id) + '">Send a test</button>' +
          '<button type="button" class="btn btn-mini btn-ghost" data-action="ops-notify-remove" data-id="' + attr(t.id) + '">Remove</button>'
      });
    });
    return uiHead("Alerts", list.length, '<button type="button" class="btn btn-mini btn-primary" data-action="ops-notify-add">Add</button>') +
      (list.length ? uiList(rows) : uiEmpty("Nobody is told yet", "Tell the inbox, Slack, Discord or a webhook when a deploy finishes or fails, or a cron job fails.")) ;
  }

  // --------------------------------------------------------------------- php

  function opsPhp(d, withSites) {
    withSites = withSites !== false;
    var s = d[0] || {}, x = d[1] || {};
    var versions = (s.versions || []).map(function (v) {
      var vals = v.values || {};
      var keys = ["memory_limit", "upload_max_filesize", "post_max_size", "max_execution_time"];
      return uiRow({
        title: "PHP " + esc(v.version), sub: keys.map(function (k) { return esc(k) + " " + esc(vals[k] || "default"); }).join(" · "), wrap: true,
        actions: '<button type="button" class="btn btn-mini btn-ghost" data-action="ops-php-value" data-series="' + attr(v.series) + '">Set a value</button>'
      });
    });
    var profile = '<select class="select compact" data-change="ops-php-profile"><option value="local"' + (s.profile === "local" ? " selected" : "") + '>Local (generous)</option><option value="production"' + (s.profile === "production" ? " selected" : "") + ">Production (like a Ploi server)</option></select>";
    var workers = '<select class="select compact" data-change="ops-php-workers">' + [1, 2, 4, 6, 8, 12, 16].map(function (n) { return '<option value="' + n + '"' + (s.workers === n ? " selected" : "") + ">" + n + (n === 1 ? " worker" : " workers") + "</option>"; }).join("") + "</select>";
    var exts = (x.extensions || []).filter(function (e) { return e.shipped; }).map(function (e) {
      return '<button type="button" class="chip' + (e.enabled ? " chip-on" : "") + '" data-action="ops-ext" data-name="' + attr(e.name) + '" data-on="' + (e.enabled ? "0" : "1") + '" aria-pressed="' + (e.enabled ? "true" : "false") + '">' + esc(e.name) + "</button>";
    }).join("");
    var siteRows = (opsSiteNames() || []).map(function (n) {
      var own = ((s.site_profiles || []).filter(function (p) { return p.site === n; })[0] || {}).profile || "";
      return uiRow({
        title: esc(n), sub: own ? "runs with the " + esc(own) + " limits" : "follows the machine (" + esc(s.profile) + ")", hover: true,
        actions: '<select class="select compact" data-change="ops-site-profile" data-site="' + attr(n) + '"><option value=""' + (own ? "" : " selected") + '>Follow the machine</option><option value="local"' + (own === "local" ? " selected" : "") + '>Local limits</option><option value="production"' + (own === "production" ? " selected" : "") + ">Production limits</option></select>"
      });
    });
    return uiHead("Limits", null, profile + workers) + uiList(versions) +
      '<p class="dim ops-note">The profile sets memory, upload size and time limits for every PHP. Workers are how many PHP processes serve each version at once.</p>' +
      (withSites ? uiHead("Sites", null, "") + uiList(siteRows) +
        '<p class="dim ops-note">A site can run with the other profile (for example production limits, to try it the way your server runs it). It is then served by a second pool of PHP processes.</p>' : "") +
      uiHead("Extensions", (x.extensions || []).filter(function (e) { return e.enabled; }).length + " on", "") +
      '<div class="chips">' + (exts || '<span class="dim">No PHP is installed.</span>') + "</div>" +
      '<p class="dim ops-note">Changing a setting restarts PHP; requests in flight are dropped.</p>';
  }

  // -------------------------------------------------------------------- ploi

  function opsPloi(d) {
    var st = d[0] || {};
    if (!st.connected) {
      return uiHead("Ploi", null, "") +
        uiEmpty(st.error ? "Ploi refused the token" : "Not connected", st.error || "Give devd an API token from ploi.io/profile/api-keys and it can show your servers, compare environments and deploy. Give the token only the scopes you want agents to use.",
          '<button type="button" class="btn btn-primary" data-action="ops-ploi-connect">Connect Ploi</button>');
    }
    var servers = OPS.data.ploiServers || [];
    var rows = servers.map(function (s) {
      return uiRow({
        dot: /active/i.test(s.status || "") ? "ok" : "warn", title: esc(s.name), sub: esc(s.ip) + " · PHP " + esc(s.php) + " · " + esc(s.sites) + " sites", hover: true,
        actions: '<button type="button" class="btn btn-mini" data-action="ops-ploi-sites" data-server="' + attr(s.id) + '">Sites</button>'
      });
    });
    var sites = (OPS.ploiSites || []).map(function (x) {
      return uiRow({
        dot: x.status === "active" ? "ok" : "warn", title: esc(x.domain), sub: "PHP " + esc(x.php) + " · " + esc(x.project_type || "") + (x.last_deploy_at ? " · deployed " + esc(x.last_deploy_at) : ""), hover: true,
        actions: '<button type="button" class="btn btn-mini" data-action="ops-ploi-link" data-server="' + attr(OPS.ploiServer) + '" data-site="' + attr(x.id) + '">Link to a local site</button>' +
          '<button type="button" class="btn btn-mini btn-ghost" data-action="ops-ploi-deploy" data-server="' + attr(OPS.ploiServer) + '" data-site="' + attr(x.id) + '" data-domain="' + attr(x.domain) + '">Deploy</button>'
      });
    });
    var who = st.user ? esc(st.user.name || st.user.email || "") : "";
    return uiHead("Ploi", who, '<button type="button" class="btn btn-mini btn-ghost" data-action="ops-ploi-disconnect">Disconnect</button>') +
      (rows.length ? uiList(rows) : uiEmpty("No servers", "")) +
      (OPS.ploiSites ? uiHead("Sites on the server", OPS.ploiSites.length, "") + (sites.length ? uiList(sites) : uiEmpty("No sites", "")) : "") +
      '<p class="dim ops-note">Deploying changes a live server, so devd asks you first. Link a site and `Compare…` in Environment shows how its .env differs from production.</p>';
  }


  // ------------------------------------------------------------ first-run setup
  //
  // What a first run installs is the person's choice, with what each thing is and how big it is shown
  // before anything downloads. It runs the same capabilities `devd setup` does, one after another, so
  // each is audited and each asks for approval where it needs it.

  var SETUP_SERVICES = [
    { name: "mariadb", label: "MariaDB", note: "the database, MySQL-compatible", size: "about 100 MB", on: true },
    { name: "redis", label: "Redis", note: "cache, queues and sessions (Redis 8 from redis-windows)", size: "about 14 MB", on: false },
    { name: "memcached", label: "memcached", note: "an in-memory cache", size: "about 4 MB", on: false },
    { name: "mailpit", label: "Mailpit", note: "catches the mail your apps send", size: "about 12 MB", on: false }
  ];

  function setupForm() {
    var svc = SETUP_SERVICES.map(function (s) {
      return '<label class="setup-choice"><input type="checkbox" id="setup-' + s.name + '"' + (s.on ? " checked" : "") + "> <span><b>" + esc(s.label) + "</b> <span class=\"dim\">" + esc(s.size) + "</span><br><span class=\"dim\">" + esc(s.note) + "</span></span></label>";
    }).join("");
    return '<p class="dim">devd always installs PHP and the web server (nginx). Choose what else you want; anything can be added later from Services.</p>' +
      '<label class="setup-choice"><input type="checkbox" checked disabled> <span><b>PHP</b> <span class="dim">about 35 MB</span><br><span class="dim">from php.net, checked against the digest it publishes</span></span></label>' +
      '<label class="setup-choice"><input type="checkbox" checked disabled> <span><b>nginx</b> <span class="dim">about 2 MB</span><br><span class="dim">the web server that fronts every site</span></span></label>' +
      svc +
      '<div class="setup-row"><label for="setup-php">PHP version</label><select id="setup-php" class="select compact"><option value="latest">Latest stable</option><option value="8.4">8.4</option><option value="8.3">8.3</option><option value="8.2">8.2</option></select></div>' +
      '<div class="setup-row"><label for="setup-profile">PHP limits</label><select id="setup-profile" class="select compact"><option value="local">Generous (local): a big import is never cut off</option><option value="production">Like a typical server (production): 128M memory, 5M uploads, 30 s</option></select></div>' +
      '<div class="inline" style="margin-top:16px"><button type="button" class="btn btn-primary" data-action="setup-go">Install</button></div>';
  }

  function setupProgress(steps) {
    return '<div class="setup-steps">' + steps.map(function (s) {
      var mark = s.state === "done" ? "✓" : s.state === "failed" ? "✗" : s.state === "running" ? "…" : "·";
      return '<div class="setup-step setup-' + s.state + '"><span class="setup-mark">' + mark + "</span> " + esc(s.label) + (s.note ? '<div class="dim">' + esc(s.note) + "</div>" : "") + "</div>";
    }).join("") + "</div>";
  }

  function setupRun(plan) {
    var steps = plan.map(function (p) { return { label: p.label, state: "waiting", note: "" }; });
    function show() { uiSheet("Setting up devd", setupProgress(steps)); }
    function next(i) {
      if (i >= plan.length) {
        steps.push({ label: "Done. Open a site from Sites to look after it, or create one.", state: "done", note: "" });
        show();
        return loadServer().then(render);
      }
      steps[i].state = "running";
      show();
      return plan[i].go().then(function (o) {
        if (!o) { steps[i].state = "failed"; steps[i].note = "stopped here; nothing after it ran"; show(); return loadServer().then(render); }
        steps[i].state = "done";
        return next(i + 1);
      });
    }
    return next(0);
  }

  /* Something already installed is a step that is already done, not an error that stops the rest. */
  function skipIf(have, go) { return have ? Promise.resolve({ ok: true }) : go(); }

  function setupStart() {
    var chosen = SETUP_SERVICES.filter(function (s) { return checked("setup-" + s.name); });
    var php = (el("setup-php") && el("setup-php").value) || "latest";
    var profile = (el("setup-profile") && el("setup-profile").value) || "local";
    var web = (state.session && state.session.settings && state.session.settings.web_server) || "nginx";
    var plan = [
      { label: "Install PHP " + (php === "latest" ? "(latest stable)" : php), go: function () { return skipIf((state.server.runtimes || []).some(function (r) { return r.runtime && r.runtime.kind === "php"; }), function () { return run("runtime.install", { kind: "php", version: php }); }); } },
      { label: "Install " + web, go: function () { return skipIf(managedService(web), function () { return run("service.install", { name: web }); }); } }
    ];
    chosen.forEach(function (s) {
      plan.push({ label: "Install " + s.label, go: function () { return skipIf(managedService(s.name), function () { return run("service.install", { name: s.name }); }); } });
    });
    if (profile !== "local") plan.push({ label: "Use the " + profile + " PHP limits", go: function () { return run("php.configure", { profile: profile }); } });
    plan.push({ label: "Start everything", go: function () { return run("stack.up", {}); } });
    if (!(state.server.sites || []).length) plan.push({ label: "Create the first site", go: function () { return run("site.create", { name: "welcome", host: "localhost" }); } });
    chosen.filter(function (s) { return s.name !== "mariadb"; }).forEach(function (s) {
      plan.push({ label: "Start " + s.label, go: function () { return run("service.start", { name: s.name }); } });
    });
    return setupRun(plan);
  }


  // ------------------------------------------------- operations: laravel, databases, nginx, health, servers, keys

  function opsSheetText(title, text) {
    uiSheet(title, '<pre class="log-block">' + esc(text) + "</pre>");
  }

  function opsLaravel(d) {
    var s = d[0] || {};
    if (!s.is_laravel) return uiEmpty("This isn't a Laravel project", "There is no artisan file in this folder, so there is nothing to run here.");
    var debugWarn = s.app_debug === "true" && s.app_env === "production";
    function lrow(dot, name, state, detail, action) {
      return "<tr><td>" + opsDot(dot) + "<b>" + esc(name) + "</b></td><td>" + esc(state) + '</td><td class="cell-note">' + detail + '</td><td class="cell-actions">' + (action || "") + "</td></tr>";
    }
    var rows = [
      lrow(s.maintenance ? "warn" : "ok", "The site", s.maintenance ? "In maintenance mode" : "Up",
        s.maintenance ? (s.maintenance_bypass_path ? "bypass path " + esc(s.maintenance_bypass_path) : "visitors see the maintenance page") : "visitors see the site",
        s.maintenance ? '<button type="button" class="btn btn-mini btn-primary" data-action="ops-lv-up">Bring it back</button>' : '<button type="button" class="btn btn-mini" data-action="ops-lv-down">Maintenance mode</button>'),
      lrow(s.storage_linked ? "ok" : "warn", "Storage link", s.storage_linked ? "Linked" : "Not linked",
        s.storage_linked ? "public/storage points at storage/app/public" : "uploaded files are not served until it is linked",
        s.storage_linked ? "" : '<button type="button" class="btn btn-mini btn-primary" data-action="ops-lv-storage">Link storage</button>'),
      lrow(debugWarn ? "bad" : "off", "Environment", esc(s.app_env || "not set"),
        "APP_DEBUG " + esc(s.app_debug || "not set") + (debugWarn ? ' · <span class="cell-bad">debug is on in production</span>' : ""),
        '<button type="button" class="btn btn-mini btn-ghost" data-action="ops-tab" data-tab="env">Open .env</button>'),
      lrow("off", "Caches", s.config_cached || s.routes_cached ? "Some cached" : "Not cached",
        (s.config_cached ? "configuration cached" : "configuration not cached") + " · " + (s.routes_cached ? "routes cached" : "routes not cached"),
        '<button type="button" class="btn btn-mini" data-action="ops-lv-clear">Clear…</button>')
    ];
    return uiHead("Laravel", null, '<button type="button" class="btn btn-mini btn-primary" data-action="ops-lv-artisan">Run an artisan command…</button>') + opsTable([{ name: "Check" }, { name: "State" }, { name: "Detail" }], rows, "Laravel status") +
      '<p class="dim ops-note">Commands run with this site\'s PHP in its folder, as an argument list and never through a shell. Clearing the application cache empties whatever the cache driver holds, so it is only done when you name it.</p>';
  }

  function opsDb(d) {
    var users = (d[0] && d[0].users) || [], backups = (d[1] && d[1].schedules) || [];
    var userRows = users.map(function (u) {
      return '<tr><td class="cell-mono"><b>' + esc(u.user) + "</b></td><td>" + esc(u.host) + '</td><td class="cell-actions">' +
        '<button type="button" class="btn btn-mini btn-ghost" data-action="ops-db-user-drop" data-user="' + attr(u.user) + '" data-host="' + attr(u.host) + '">Drop</button></td></tr>';
    });
    var backupRows = backups.map(function (b) {
      var dot = b.enabled === false ? "off" : b.last_ok === false ? "bad" : "ok";
      var last = b.last_run_at ? esc(shortTime(b.last_run_at)) + (b.last_ok === false ? ' <span class="cell-bad">failed: ' + esc(b.last_error || "") + "</span>" : "") : '<span class="dim">has not run</span>';
      return "<tr><td>" + opsDot(dot) + "<b>" + esc(b.database) + "</b></td><td>" + esc(b.schedule) + "</td><td>" + last + "</td><td>" + (b.next_run_at ? esc(shortTime(b.next_run_at)) : '<span class="dim">none</span>') + "</td><td>" + esc(String(b.keep)) + " newest</td>" +
        '<td class="cell-actions"><button type="button" class="btn btn-mini" data-action="ops-backup-run" data-id="' + attr(b.id) + '">Back up now</button>' +
        '<button type="button" class="btn btn-mini btn-ghost" data-action="ops-backup-files" data-db="' + attr(b.database) + '">Files</button>' +
        '<button type="button" class="btn btn-mini btn-ghost" data-action="ops-backup-remove" data-id="' + attr(b.id) + '">Stop</button></td></tr>';
    });
    return uiHead("Database users", users.length, '<button type="button" class="btn btn-mini btn-primary" data-action="ops-db-user-add">Add user</button>') +
      (userRows.length ? opsTable([{ name: "User" }, { name: "From" }], userRows, "Database users") : uiEmpty("No users yet", "Apps connect as root now. A user with its own password and one database is what a server has.")) +
      uiHead("Backups", backups.length, '<button type="button" class="btn btn-mini btn-primary" data-action="ops-backup-add">Back up a database</button>') +
      (backupRows.length ? opsTable([{ name: "Database" }, { name: "Schedule" }, { name: "Last run" }, { name: "Next run" }, { name: "Keeps" }], backupRows, "Backups") : uiEmpty("No backups scheduled", "devd dumps a database on a schedule while it is open, keeps the newest few, and tells you when one fails.")) +
      '<p class="dim ops-note">A backup is a plain SQL file under the devd backups folder. Restore one by giving its path to <code>db.restore</code>.</p>';
  }

  function opsNginx(d) {
    var s = d[0] || {}, presets = (d[1] && d[1].presets) || [];
    var mine = s.directives;
    var body = uiHead("Your directives", null, '<button type="button" class="btn btn-mini btn-primary" data-action="ops-nx-edit">' + (mine ? "Edit" : "Add") + '</button>' + (mine ? '<button type="button" class="btn btn-mini btn-ghost" data-action="ops-nx-clear">Remove</button>' : "")) +
      (mine ? '<pre class="log-block">' + esc(mine) + "</pre>" : uiEmpty("None", "devd generates this site's server block. What it cannot know (a bigger upload limit, CORS headers, a cache rule) goes here."));
    body += uiHead("Add a preset", null, "") + '<div class="chips">' + presets.map(function (p) {
      return '<button type="button" class="chip" data-action="ops-nx-preset" data-name="' + attr(p.name) + '" title="' + attr(p.does) + '">' + esc(p.name) + "</button>";
    }).join("") + "</div>";
    body += uiHead("Generated", null, '<button type="button" class="btn btn-mini btn-ghost" data-action="ops-nx-generated">Show the server block</button>');
    body += '<p class="dim ops-note">Directives are checked with <code>nginx -t</code> before they are kept; if nginx rejects them the previous ones are put back. ' + (s.web_server === "apache" ? "The Apache driver does not use them." : "") + "</p>";
    return body;
  }

  function opsHealth(d) {
    var checks = (d[0] && d[0].checks) || [], m = d[1] || {};
    var mem = m.memory || {};
    var disks = (m.disks || []).map(function (x) { return esc(x.mount) + " " + esc(String(x.free_gb)) + " GB free of " + esc(String(x.total_gb)) + " GB"; }).join(" · ");
    var machine = uiList([
      uiRow({ dot: "off", title: "This machine", sub: esc([m.cpu_percent != null ? "CPU " + Math.round(m.cpu_percent) + "%" : (m.load_average != null ? "load " + m.load_average : ""), mem.total_mb ? "memory " + Math.round(mem.used_mb / 1024 * 10) / 10 + " of " + Math.round(mem.total_mb / 1024 * 10) / 10 + " GB" : "", m.uptime_hours != null ? "up " + m.uptime_hours + " h" : "", m.devd ? m.devd.running + "/" + m.devd.services + " devd services" : ""].filter(Boolean).join(" · ")), wrap: true }),
      disks ? uiRow({ dot: "off", title: "Disks", sub: disks, wrap: true }) : ""
    ].filter(Boolean));
    var rows = checks.map(function (c) {
      return uiRow({
        dot: c.state === "up" ? "ok" : c.state === "down" ? "bad" : "off", title: esc(c.site) + ' <span class="pill">' + esc(c.state || "unknown") + "</span>",
        sub: (c.last_at ? "asked " + esc(shortTime(c.last_at)) : "not asked yet") + (c.last_status ? " · answered " + c.last_status + " in " + c.last_ms + " ms" : "") + (c.last_error ? " · " + esc(c.last_error) : "") + (c.url ? " · " + esc(c.url) : " · through devd's own web server"), wrap: true, hover: true,
        actions: '<button type="button" class="btn btn-mini" data-action="ops-health-check" data-site="' + attr(c.site) + '">Ask now</button><button type="button" class="btn btn-mini btn-ghost" data-action="ops-health-remove" data-site="' + attr(c.site) + '">Stop</button>'
      });
    });
    return uiHead("Machine", null, "") + machine +
      uiHead("Site health", checks.length, '<button type="button" class="btn btn-mini btn-primary" data-action="ops-health-add">Watch a site</button>') +
      (rows.length ? uiList(rows) : uiEmpty("No sites are watched", "devd asks the site for its front page every minute while it is open and tells the alert targets when it goes down and when it is back.")) ;
  }

  function opsServers(d) {
    var list = (d[0] && d[0].servers) || [], links = (d[1] && d[1].links) || [];
    var rows = list.map(function (s) {
      var n = attr(s.name);
      return "<tr><td>" + opsDot(s.host_key_pinned ? "ok" : "warn") + "<b>" + esc(s.name) + '</b> <span class="pill">' + esc(s.provider) + "</span></td>" +
        '<td class="cell-mono">' + esc(s.user + "@" + s.host + (s.port !== 22 ? ":" + s.port : "")) + "</td>" +
        '<td class="cell-mono">' + esc(s.key) + "</td>" +
        "<td>" + (s.host_key_pinned ? "Pinned" : '<span class="cell-warn">Not connected yet</span>') + "</td>" +
        "<td>" + (s.last_ok_at ? esc(shortTime(s.last_ok_at)) : '<span class="dim">never</span>') + "</td>" +
        '<td class="cell-actions"><button type="button" class="btn btn-mini' + (s.host_key_pinned ? "" : " btn-primary") + '" data-action="ops-srv-connect" data-name="' + n + '">' + (s.host_key_pinned ? "Test" : "Connect") + "</button>" +
          '<button type="button" class="btn btn-mini btn-ghost" data-action="ops-srv-info" data-name="' + n + '">Look</button>' +
          '<button type="button" class="btn btn-mini btn-ghost" data-action="ops-srv-provision" data-name="' + n + '">Set up for PHP</button>' +
          '<button type="button" class="btn btn-mini btn-ghost" data-action="ops-srv-run" data-name="' + n + '">Run…</button>' +
          '<button type="button" class="btn btn-mini btn-ghost" data-action="ops-srv-remove" data-name="' + n + '">Forget</button></td></tr>';
    });
    var linkRows = links.map(function (l) {
      return "<tr><td><b>" + esc(l.site) + "</b></td><td>" + esc(l.server) + "</td><td>" + esc(l.mode) + '</td><td class="cell-mono">' + esc(l.path) + (l.domain ? " · " + esc(l.domain) : "") + "</td>" +
        '<td class="cell-actions"><button type="button" class="btn btn-mini btn-ghost" data-action="ops-link-remove" data-id="' + attr(l.id) + '">Unlink</button></td></tr>';
    });
    return uiHead("Servers", list.length, '<button type="button" class="btn btn-mini btn-primary" data-action="ops-srv-add">Add a server</button>') +
      (rows.length ? opsTable([{ name: "Server" }, { name: "Address" }, { name: "Key" }, { name: "Host key" }, { name: "Last answered" }], rows, "Servers") : uiEmpty("No servers yet", "Any machine you can SSH into: DigitalOcean, Hetzner, Vultr, a Ploi or Forge server, a Raspberry Pi. Add it, connect once to pin its host key, and deploy to it from the Deploy tab.")) +
      uiHead("Where sites deploy", links.length, "") + (linkRows.length ? opsTable([{ name: "Site" }, { name: "Server" }, { name: "Mode" }, { name: "Path" }], linkRows, "Where sites deploy") : uiEmpty("No site is linked to a server", "Open a site's Deploy tab and choose Link to a server.")) +
      '<p class="dim ops-note">devd reaches servers with the key you name and pins each server\'s host key in its own file, never your known_hosts. Setting a server up and deploying are approved by you each time.</p>';
  }

  function opsKeys(d) {
    var keys = (d[0] && d[0].keys) || [];
    var rows = keys.map(function (k) {
      return '<tr><td class="cell-mono"><b>' + esc(k.name) + "</b></td><td>" + esc(k.type || "") + "</td><td>" + (k.has_private_key ? "Private key here" : '<span class="cell-warn">No private key here</span>') + '</td><td class="cell-note">' + esc(k.comment || "") + "</td>" +
        '<td class="cell-actions"><button type="button" class="btn btn-mini" data-action="ops-key-public" data-name="' + attr(k.name) + '">Show public key</button></td></tr>';
    });
    return uiHead("SSH keys", keys.length, '<button type="button" class="btn btn-mini btn-primary" data-action="ops-key-new">Make a key</button><button type="button" class="btn btn-mini" data-action="ops-key-adopt">Take in a key</button>') +
      (rows.length ? opsTable([{ name: "Key" }, { name: "Type" }, { name: "Private half" }, { name: "Comment" }], rows, "SSH keys") : uiEmpty("No keys in ~/.ssh", "Make one, or take in a private key you already have. The public half is what a server's authorized_keys and GitHub want.")) +
      '<p class="dim ops-note">Private keys are never shown. A new key has no passphrase; add one with <code>ssh-keygen -p</code> if you want one.</p>';
  }

  /* What the Deploy tab shows beyond the history: servers, variables, scheduled deploys, the staging site. */
  function opsDeployExtras(d) {
    var links = (d[4] && d[4].links) || [], vars = (d[5] && d[5].vars) || {}, sched = ((d[6] && d[6].scheduled) || []).filter(function (x) { return x.state === "pending"; });
    var staging = ((d[7] && d[7].staging) || []).filter(function (x) { return x.of === OPS.site; })[0];
    var nv = Object.keys(vars).length;
    var linkRows = links.map(function (l) {
      var id = attr(l.id);
      return uiRow({
        dot: "ok", title: "Deploys to " + esc(l.server) + ' <span class="pill">' + esc(l.mode) + "</span>",
        sub: '<span class="mono">' + esc(l.path) + "</span>" + (l.domain ? " · " + esc(l.domain) : ""), wrap: true, hover: true,
        actions: '<button type="button" class="btn btn-mini btn-primary" data-action="ops-remote-deploy" data-link="' + id + '">Deploy</button>' +
          (l.mode === "releases" ? '<button type="button" class="btn btn-mini btn-ghost" data-action="ops-remote-releases" data-link="' + id + '">Releases</button><button type="button" class="btn btn-mini btn-ghost" data-action="ops-remote-rollback" data-link="' + id + '">Roll back</button>' : "") +
          '<button type="button" class="btn btn-mini btn-ghost" data-action="ops-remote-env" data-link="' + id + '">Send .env</button>' +
          (l.domain ? '<button type="button" class="btn btn-mini btn-ghost" data-action="ops-remote-site" data-link="' + id + '">Create site</button><button type="button" class="btn btn-mini btn-ghost" data-action="ops-remote-ssl" data-link="' + id + '">HTTPS</button>' : "") +
          '<button type="button" class="btn btn-mini btn-ghost" data-action="ops-link-remove" data-id="' + id + '">Unlink</button>'
      });
    });
    var more = [
      uiRow({ dot: nv ? "ok" : "off", title: "Variables", sub: nv ? esc(Object.keys(vars).join(", ")) : "none: values a deploy gets as {NAME} and as environment variables", wrap: true,
        actions: '<button type="button" class="btn btn-mini" data-action="ops-vars-edit">Edit…</button>' }),
      uiRow({ dot: sched.length ? "warn" : "off", title: "Scheduled", sub: sched.length ? sched.map(function (x) { return esc(shortTime(x.at)); }).join(", ") : "nothing is scheduled", wrap: true,
        actions: '<button type="button" class="btn btn-mini" data-action="ops-sched-add">Deploy later…</button>' }),
      uiRow({ dot: staging ? "ok" : "off", title: "Staging", sub: staging ? esc(staging.staging_site) + " on branch " + esc(staging.branch) : "a second copy of this site on its own branch", wrap: true,
        actions: staging ? '<button type="button" class="btn btn-mini btn-primary" data-action="ops-staging-promote">Ship to production</button><button type="button" class="btn btn-mini btn-ghost" data-action="ops-staging-remove">Remove</button>' : '<button type="button" class="btn btn-mini" data-action="ops-staging-create">Make a staging site</button>' })
    ];
    var schedRows = sched.map(function (x) {
      return uiRow({ dot: "warn", title: "Deploy at " + esc(shortTime(x.at)), sub: esc(x.note || ""), actions: '<button type="button" class="btn btn-mini btn-ghost" data-action="ops-sched-cancel" data-id="' + attr(x.id) + '">Cancel</button>' });
    });
    return uiHead("Servers", links.length, '<button type="button" class="btn btn-mini" data-action="ops-link-add">Link to a server</button>') +
      (linkRows.length ? uiList(linkRows) : '<p class="dim ops-note">Not linked to a server. Link it, and Deploy ships it over SSH.</p>') +
      uiList(more) + (schedRows.length ? uiList(schedRows) : "");
  }


  function managedServicePid(pid) {
    return (state.server.services || []).some(function (x) { return x.pid === pid; });
  }

  function inventoryRows() {
    var inv = state.server.inventory || [];
    return inv.filter(function (v) {
      // devd's MariaDB runs as mysqld.exe on Windows. The inventory sees the
      // port owner but does not know devd started it, so without this it shows
      // a second MariaDB/MySQL under "Already on this computer".
      if (v.port_owner && v.port_owner.pid && managedServicePid(v.port_owner.pid)) return false;
      // Programs that are not on this machine belong under Available, not in
      // the list of things already here.
      return !(v.status === "missing" && !v.path && !v.port_owner);
    }).map(function (v) {
      var rec = v.recommendation || {};
      var stateLabel = INV_LABEL[v.status] || v.status;
      if (v.status === "missing" && v.port_owner) stateLabel = "Not found on PATH";
      var opts = v.options.filter(function (o) { return o.available && o.action !== "ignore"; });
      function label(o) { return o.action === "adopt" ? "Use it" : o.action === "watch" ? "Only watch" : o.action === "install" ? "Install" : o.action === "remove" ? "Remove" : o.action; }
      var btns = opts.map(function (o) {
        var recd = o.action === rec.action;
        var cls = "btn btn-mini" + (recd ? " btn-primary" : o.risk === "destructive" ? " btn-danger" : " btn-ghost");
        return '<button type="button" class="' + cls + '" data-action="svc-option" data-tool="' + attr(v.tool) + '" data-opt="' + attr(o.action) + '" title="' + attr((o.why_not || "") + (o.risk === "approval" ? " Asks you first." : "")) + '">' + esc(label(o)) + "</button>";
      }).join("");
      var own = { redis: "redis", postgresql: "postgres", mongodb: "mongodb", memcached: "memcached" }[v.tool];
      if (own && v.path && !managedService(own)) {
        btns = '<button type="button" class="btn btn-mini btn-primary" data-action="adopt-found" data-name="' + attr(own) + '" data-path="' + attr(v.path.replace(/[\\/][^\\/]*$/, "")) + '">Run it with devd</button>';
      }
      if (v.decision) btns += '<button type="button" class="btn btn-mini btn-ghost" data-action="svc-option" data-tool="' + attr(v.tool) + '" data-opt="clear">Reset</button>';
      return uiRow({
        dot: v.status === "ready" ? "ok" : v.status === "installed_stopped" ? "warn" : v.status === "missing" ? "off" : "bad",
        title: brandImg(v.tool, "brand-dot") + esc(v.name) + (v.decision ? ' <span class="pill pill-accent">' + esc(v.decision.mode) + "</span>" : "") +
          (v.required ? ' <span class="pill pill-warn">needed by ' + v.required_by.length + "</span>" : ""),
        sub: esc(stateLabel + (v.path ? " · " + v.path : "") + (v.port_owner && v.port_owner.program ? " · port held by " + v.port_owner.program : "")),
        actions: btns
      });
    });
  }

  function renderServices() {
    if (state.view !== "services") return;
    var rows = state.server.services;
    var byName = {};
    rows.forEach(function (r) { byName[r.name] = r; });
    var counts = serviceCounts(rows);
    var up = counts.up;
    var extern = [];
    var installed = serviceUnits(rows).map(function (unit) {
      var isExt = unit.rows ? unit.rows.some(function (r) { return !r.running && r.listening; }) : (!unit.row.running && unit.row.listening);
      var html = serviceRowHtml(unit);
      if (isExt) extern.push(html);
      return isExt ? "" : html;
    }).filter(Boolean);
    function serviceRowHtml(unit) {
      if (unit.rows) return phpUnitRow(unit, false);
      var svc = unit.row;
      var activeWeb = (state.session && state.session.settings && state.session.settings.web_server) || "nginx";
      var webControl = "";
      if (svc.name === "nginx" || svc.name === "apache") {
        webControl = activeWeb === svc.name
          ? '<span class="pill pill-accent">serving sites</span>'
          : (svc.installed ? '<button type="button" class="btn btn-mini" data-action="web-use" data-name="' + attr(svc.name) + '">Use for sites</button>' : "");
      }
      var ladder = serviceLadder(svc);
      var where = [servicePorts(svc), svc.running && svc.pid ? "pid " + svc.pid : ""].filter(Boolean).join(" · ");
      return uiRow({
        dot: ladder.dot,
        title: brandImg(svc.kind || svc.name, "brand-dot") + '<span class="row-name">' + esc(svc.name) + "</span>" + (svc.version ? ' <span class="fact mono">' + esc(svc.version) + "</span>" : "") +
          ' <span class="facts">' + ladder.facts.join("") + "</span>",
        sub: (where ? '<span class="mono">' + esc(where) + "</span>" : "") +
          (ladder.detail ? (where ? " · " : "") + esc(ladder.detail) : "") +
          (!where && !ladder.detail ? esc(svc.installed ? "managed by devd" : "recorded, but its files are not where devd installed them") : "") +
          (serviceDependents(svc) ? '<div class="ui-row-dep">' + esc(serviceDependents(svc)) + "</div>" : ""),
        wrap: !!ladder.detail || !!serviceDependents(svc),
        actions: webControl + startupToggle(svc) + (svc.running
          ? '<button type="button" class="btn btn-mini" data-action="stop-service" data-name="' + attr(svc.name) + '">Stop</button>'
          : svc.installed
            ? '<button type="button" class="btn btn-mini btn-primary" data-action="start-service" data-name="' + attr(svc.name) + '">Start</button>'
            : '<button type="button" class="btn btn-mini btn-primary" data-action="install-service" data-name="' + attr(svc.name) + '">Install</button>') +
          (svc.running ? '<button type="button" class="btn btn-mini btn-ghost" data-action="restart-service" data-name="' + attr(svc.name) + '">Restart</button>' : "") +
          '<button type="button" class="btn btn-mini btn-ghost" data-action="service-logs" data-name="' + attr(svc.name) + '">Logs</button>' +
          '<button type="button" class="btn btn-mini btn-ghost" data-action="remove-service" data-name="' + attr(svc.name) + '">Remove</button>',
        hover: !svc.running && svc.installed ? true : true
      });
    }
    var found = inventoryRows();
    var missing = KNOWN_SERVICES.filter(function (k) { return !byName[k.name] && !(k.name === "php-cgi" && rows.some(function (r) { return /^php-cgi/.test(r.name); })); })
      .filter(function (k) {
        var tool = k.name === "postgres" ? "postgresql" : k.name;
        return !k.find || k.name === "apache" || !(state.server.inventory || []).some(function (v) { return v.tool === tool && v.path; });
      })
      .map(function (k) {
        return uiRow({
          dot: "off", title: esc(k.label), sub: esc(k.note),
          actions: k.name === "php-cgi"
            ? '<button type="button" class="btn btn-mini" data-action="nav" data-view="runtimes">Install a PHP runtime</button>'
            : (k.download === false ? "" : '<button type="button" class="btn btn-mini btn-primary" data-action="install-service" data-name="' + attr(k.name) + '">Install</button>')
        });
      });
    var body = summaryBand([
      (function () {
        var rows = state.server.services || [];
        var run = rows.filter(function (x) { return x.running; });
        var silent = run.filter(function (x) { return x.ready === false; }).length;
        var answered = run.filter(function (x) { return x.ready === true; }).length;
        var held = rows.filter(function (x) { return !x.running && x.listening; }).length;
        var bad = silent || held;
        return { label: "Stack", value: counts.total ? up + " of " + counts.total + " running" : "Not installed",
          detail: silent ? silent + " running but not answering" : held ? held + (held === 1 ? " port is" : " ports are") + " held by programs devd did not start" :
            counts.total && up === counts.total ? (answered === run.length ? "every process answered its protocol" : answered + " of " + run.length + " processes confirmed answering") :
            counts.total ? "start services when a site needs them" : "install a web server, database and PHP",
          tone: bad ? "warn" : counts.total && up === counts.total ? "ok" : up ? "warn" : "", dot: bad ? "warn" : counts.total && up === counts.total ? "ok" : up ? "warn" : "off" };
      })(),
      { label: "Managed here", value: String(counts.total), detail: counts.total === 1 ? "service devd supervises" : "services devd supervises" },
      { label: "Available", value: String(missing.length), detail: missing.length ? "ready to install or adopt" : "nothing else to install" }
    ]);
    body += (extern.length
      ? uiHead("Port held by another program", extern.length) +
        '<div class="ui-more" style="margin-top:0">devd did not start whatever is listening on these ports, so it does not control it and will not start its own copy on top. Stop the other program, or give devd\u2019s service another port.</div>' + uiList(extern)
      : "") +
      (installed.length || !extern.length ? uiHead("Managed by devd", counts.total ? up + " of " + counts.total + " running" : null) : "") +
      (installed.length ? uiList(installed) : extern.length ? "" : serverFailed("service.list") ? uiEmpty("Couldn't read your services", serverFailed("service.list") + ".", '<button type="button" class="btn" data-action="refresh">Try again</button>') : uiEmpty("Nothing installed yet", "Install a web server, MariaDB and a PHP runtime to serve sites.")) +
      (installed.length || extern.length ? '<div class="ui-more"><b>Running</b> means devd found the process it started. <b>Answers</b> means the service replied in its own protocol (an HTTP request, a database handshake, a FastCGI record) - a port that is merely open is not counted as healthy.</div>' : "") +
      (installed.length || extern.length ? '<div class="ui-more">Startup means the service is included when you press Start all, or when devd starts with your account. It does not start the service now. The web server is chosen with web.use; PHP listeners follow the installed runtimes.</div>' : "");
    if (found.length) body += uiHead("Already on this computer", found.length) + uiList(found) +
      '<div class="ui-more">devd never replaces what you have. Using or watching a service only records your choice.</div>';
    if (missing.length) body += uiHead("Available", missing.length) + uiList(missing);
    el("main").innerHTML = page("Services", "",
      '<button type="button" class="btn" data-action="stack-down">Stop all</button><button type="button" class="btn btn-primary" data-action="stack-up">Start all</button>', body);
  }

  // --------------------------------------------------------------------- sites

  function siteForm() {
    return (
      '<div class="panel"><div class="panel-head"><span>New site</span></div>' +
      '<div class="panel-body">' +
      '<div class="form-grid">' +
      '<label class="field"><span class="field-label">Name</span>' +
      '<input class="input mono" id="site-name" placeholder="blog" autocomplete="off" spellcheck="false"></label>' +
      '<label class="field"><span class="field-label">Hostname, optional</span>' +
      '<input class="input mono" id="site-host" placeholder="blog.test" autocomplete="off" spellcheck="false"></label>' +
      '<label class="field"><span class="field-label">Serve a running program, optional</span>' +
      '<input class="input mono" id="site-upstream" placeholder="3000 or localhost:5173" autocomplete="off" spellcheck="false"></label>' +
      "</div>" +
      '<div class="ui-quiet">A site records an address and how it answers. HTTPS issues a local certificate for that address; a database is created inside MariaDB. Publishing the hostname and starting the stack are separate, explicit steps.</div>' +
      '<div class="inline" style="margin-top:12px">' +
      '<label class="check"><input type="checkbox" id="site-tls" checked> HTTPS with a local certificate</label>' +
      '<label class="check"><input type="checkbox" id="site-db"> Create a database</label>' +
      '<span class="grow"></span>' +
      '<button type="button" class="btn" data-action="new-site">Cancel</button>' +
      '<button type="button" class="btn btn-primary" data-action="create-site">Create site</button>' +
      "</div></div></div>"
    );
  }

  // ------------------------------------------------------------------ services


  // Services already on the machine: detected, never assumed healthy, never replaced.
  var INV_LABEL = { ready: "Ready", installed_stopped: "Installed, not running", port_conflict: "Port taken by something else", not_operational: "Not operational", limited: "Detected, limited control", missing: "Not installed" };

  function managedService(name) {
    return (state.server.services || []).some(function (x) { return x.name === name; });
  }

  // ------------------------------------------------------------------ runtimes

  /* php and composer from a terminal. devd keeps one folder of launchers; this shows whether that
     folder is on the person's PATH and lets them put it there (or take it off). */
  var SHELL = { data: null, loading: false };

  function loadShell() {
    SHELL.loading = true;
    return read("shell.status").then(function (d) { SHELL.data = d; }, function () { SHELL.data = { error: true }; }).then(function () {
      SHELL.loading = false;
      if (state.view === "runtimes") renderRuntimes();
    });
  }

  function shellPanel() {
    var d = SHELL.data;
    if (!d) {
      if (!SHELL.loading) loadShell();
      return "";
    }
    if (d.error) return "";
    var have = [];
    if (d.php_installed) have.push("php");
    if (d.composer_installed) have.push("composer");
    var sub = !have.length
      ? "Once a PHP is installed, php and composer work in any terminal."
      : d.on_path
        ? have.join(" and ") + (have.length === 1 ? " works" : " work") + " in new terminals, using the PHP of the folder you are in."
        : "New terminals will not find " + have.join(" or ") + " until devd's folder is on your PATH.";
    var actions = "";
    if (d.php_installed) {
      actions += d.on_path
        ? '<button type="button" class="btn btn-mini btn-ghost" data-action="shell-path" data-mode="disable">Take off PATH</button>'
        : '<button type="button" class="btn btn-mini btn-primary" data-action="shell-path" data-mode="enable">Add to PATH</button>';
      if (!d.composer_installed) actions += '<button type="button" class="btn btn-mini" data-action="install-composer">Install Composer</button>';
    }
    return uiHead("Command line") + uiList([uiRow({
      dot: d.on_path && have.length ? "ok" : "off",
      title: "php and composer in any terminal",
      sub: esc(sub),
      actions: actions
    })]);
  }

  function renderRuntimes() {
    if (state.view !== "runtimes") return;
    var q = state.filters.runtimes || "";
    var phpBody;
    if (OPS.data.php) {
      phpBody = uiHead("PHP settings", null, "") + opsPhp(OPS.data.php, false);
    } else {
      var failed = OPS.error && OPS.errorTab === "php";
      if (!failed) opsLoad("php");
      phpBody = uiHead("PHP settings", null, "") + (failed ? '<div class="notice notice-bad">' + esc(OPS.error) + "</div>" : '<div class="loading"></div>');
    }
    var body = RUNTIME_KINDS.map(function (kind) {
      var installed = state.server.runtimes.filter(function (r) {
        return r.runtime.kind === kind.kind && keeps(q, r.runtime.version, r.runtime.path, kind.label, kind.kind);
      });
      var rows = installed.map(function (r) {
        var rt = r.runtime;
        var own = r.managed !== false;
        var series = devdSeries(rt.version);
        var serving = kind.kind === "php" ? (state.server.sites || []).filter(function (row) {
          return row.serving === "php" && devdSeries(row.php_effective) === series;
        }) : [];
        var pinned = serving.filter(function (row) { return row.php_source === "pinned"; }).length;
        var usage = kind.kind !== "php" || !serving.length
          ? (kind.kind === "php" ? "not used by a site" : "")
          : serving.length + (serving.length === 1 ? " site" : " sites") + (pinned ? " · " + pinned + " pinned" : "");
        return uiRow({
          dot: "ok",
          title: esc(rt.version) + (r.primary ? ' <span class="pill pill-accent">default</span>' : "") + (own ? "" : ' <span class="pill">found on this computer</span>'),
          sub: '<span class="mono" title="' + attr(rt.path) + '">' + esc(rt.path) + "</span>",
          meta: usage ? esc(usage) : "",
          actions: (kind.kind === "php" && !r.primary && own ? '<button type="button" class="btn btn-mini" data-action="use-runtime" data-version="' + attr(rt.version) + '">Use for new sites</button>' : "") +
            (own ? '<button type="button" class="btn btn-mini btn-ghost" data-action="remove-runtime" data-kind="' + attr(kind.kind) + '" data-version="' + attr(rt.version) + '">Remove</button>' : ""),
          hover: true
        });
      });
      return uiHead(kind.label, installed.length || null, '<button type="button" class="btn btn-mini" data-action="install-runtime" data-kind="' + attr(kind.kind) + '">' + (installed.length ? "Install another" : "Install latest") + "</button>") +
        (rows.length ? uiList(rows) : '<div class="ui-quiet">' + esc(kind.note) + " \u2014 not installed.</div>") +
        (kind.kind === "php" ? phpBody : "");
    });
    el("main").innerHTML = page("Runtimes", "", "", shellPanel() + body.join("") +
      '<div class="ui-more">Each runtime is downloaded from its maker and checked against the digest they publish. A project can use its own version with <span class="mono">devd runtime pin</span>. PHP limits for one site are set on that site\'s page.</div>');
  }

  // -------------------------------------------------------------- certificates

  function renderCertificates() {
    if (state.view !== "certificates") return;
    var s = state.server;
    var ca = s.ca;
    var form = state.certFormOpen
      ? '<div class="panel"><div class="panel-head"><span>Issue a certificate</span></div>' +
        '<div class="panel-body"><div class="inline">' +
        '<input class="input mono grow" id="cert-host" placeholder="blog.test" spellcheck="false">' +
        '<button type="button" class="btn" data-action="cert-form">Cancel</button>' +
        '<button type="button" class="btn btn-primary" data-action="issue-cert">Issue</button>' +
        "</div></div></div>"
      : "";

    var siteHosts = {};
    (s.sites || []).forEach(function (r) { if (r.site) siteHosts[r.site.host || r.site.name] = r.site.name; });
    var missingKeys = s.certs.filter(function (c) { return !c.key_present; }).length;
    var unused = s.certs.filter(function (c) { return !siteHosts[c.host]; }).length;

    var band = summaryBand([
      { label: "Authority", value: !ca ? "None yet" : caTrusted() ? "Trusted" : "Not trusted", detail: !ca ? "an HTTPS site creates it" : caTrusted() ? "browsers accept devd's certificates" : "browsers will warn until you trust it",
        tone: ca && caTrusted() ? "ok" : "warn", dot: ca && caTrusted() ? "ok" : ca ? "warn" : "off" },
      { label: "Issued", value: s.certs.length + (s.certs.length === 1 ? " certificate" : " certificates"), detail: unused ? unused + " not used by any site" : "each one belongs to a site", dot: s.certs.length ? "ok" : "off" },
      { label: "Keys", value: missingKeys ? missingKeys + " missing" : "All present", detail: missingKeys ? "reissue those hostnames" : "every certificate has its private key", tone: missingKeys ? "bad" : "", dot: missingKeys ? "bad" : "ok" }
    ]);

    // Trust is a change to the operating system's certificate store, so its controls
    // stay explicit and say whose store they change.
    var caBlock = uiHead("Local certificate authority", null) + (ca
      ? uiList([uiRow({
          dot: caTrusted() ? "ok" : "warn",
          title: '<span class="row-name">devd authority</span> <span class="facts">' +
            (caTrusted() ? '<span class="fact fact-ok">trusted</span>' : '<span class="fact fact-warn">not trusted</span>') + "</span>",
          sub: '<span class="mono">' + esc(ca.path) + "</span><div>" + esc(caTrustLabel().charAt(0).toUpperCase() + caTrustLabel().slice(1)) + "</div>",
          wrap: true,
          actions: '<button type="button" class="btn btn-mini' + (caTrusted() ? "" : " btn-primary") + '" data-action="ca-trust" data-machine="false">Trust for this user</button>' +
            '<button type="button" class="btn btn-mini" data-action="ca-trust" data-machine="true" title="Needs an administrator">Trust for all users</button>' +
            '<button type="button" class="btn btn-mini btn-ghost" data-action="ca-untrust">Remove trust</button>'
        })]) +
        '<div class="ui-more">Trusting adds the authority to the Windows certificate store. "For this user" needs no administrator; "for all users" does.</div>'
      : uiEmpty("No certificate authority yet", "Creating an HTTPS site makes one. It never leaves this machine."));

    var rows = s.certs.map(function (c) {
      var site = siteHosts[c.host];
      return uiRow({
        dot: !c.key_present ? "bad" : site ? "ok" : "off",
        title: '<span class="row-name mono">' + esc(c.host) + '</span> <span class="facts">' +
          (c.key_present ? '<span class="fact fact-ok">key present</span>' : '<span class="fact fact-bad">key missing</span>') +
          (site ? '<span class="fact">site ' + esc(site) + "</span>" : '<span class="fact" title="No site uses this hostname">unused</span>') + "</span>",
        sub: '<span class="mono" title="' + attr(c.cert) + '">' + esc(c.cert) + "</span>",
        actions: site ? '<button type="button" class="btn btn-mini btn-ghost" data-action="open-site" data-name="' + attr(site) + '">Site</button>' : ""
      });
    });
    var issued = uiHead("Issued certificates", s.certs.length) +
      (rows.length ? uiList(rows) : uiEmpty("No certificates yet", "An HTTPS site issues one, or issue one for a hostname above."));

    el("main").innerHTML = page(
      "Certificates",
      "devd runs its own authority. Every HTTPS site gets a certificate signed by it.",
      '<button type="button" class="btn" data-action="cert-form">Issue for a hostname</button>',
      form + band + caBlock + issued
    );
  }

  // --------------------------------------------------------------------- hosts

  function renderHosts() {
    if (state.view !== "hosts") return;
    var data = state.server.hosts || { adding: [], removing: [], entries: [], file: "", changes: false };
    var adding = data.adding || [];
    var removing = data.removing || [];
    var entries = data.entries || [];
    var pending = adding.length + removing.length;

    var siteOf = {};
    (state.server.sites || []).forEach(function (r) { if (r.site) siteOf[r.site.host || r.site.name] = r.site.name; });

    var band = summaryBand([
      { label: "Published", value: entries.length + (entries.length === 1 ? " name" : " names"), detail: "in devd's managed block", dot: entries.length ? "ok" : "off" },
      { label: "Pending", value: pending ? adding.length + " to add, " + removing.length + " to remove" : "Nothing", detail: pending ? "sync to write them" : "the file matches your sites", tone: pending ? "warn" : "", dot: pending ? "warn" : "ok" },
      { label: "Hosts file", value: data.file ? "Readable" : "Not found", detail: data.file ? "devd edits only its own block" : "devd could not locate it", tone: data.file ? "" : "bad", dot: data.file ? "ok" : "bad" }
    ]);
    var head = band + '<div class="ui-more" style="margin-top:0">Writing the hosts file needs an elevated shell on Windows. devd snapshots it first and keeps every line it did not write. File: <span class="mono">' + esc(data.file) + "</span></div>";

    function entryRow(e, change) {
      var site = siteOf[e.host];
      return uiRow({
        dot: change === "add" ? "warn" : change === "remove" ? "bad" : site ? "ok" : "off",
        title: '<span class="row-name mono">' + esc(e.host) + '</span> <span class="facts">' +
          (change === "add" ? '<span class="fact fact-warn">will be added</span>' : change === "remove" ? '<span class="fact fact-bad">will be removed</span>' : '<span class="fact fact-ok">published</span>') +
          (site ? '<span class="fact">site ' + esc(site) + "</span>" : change === "remove" ? "" : '<span class="fact">no site</span>') + "</span>",
        sub: '<span class="mono">' + esc(e.address) + "</span>"
      });
    }
    var changes = pending ? uiHead("Pending changes", pending) + uiList(adding.map(function (e) { return entryRow(e, "add"); }).concat(removing.map(function (e) { return entryRow(e, "remove"); }))) : "";
    var published = uiHead("Published names", entries.length) + (entries.length
      ? uiList(entries.map(function (e) { return entryRow(e, ""); }))
      : uiEmpty("No names published", "Sites on localhost resolve without one."));

    el("main").innerHTML = page(
      "Hosts",
      "Names on .test resolve through a managed block in the system hosts file.",
      '<button type="button" class="btn btn-primary" data-action="sync-hosts"' +
        (pending ? "" : " disabled") + ">Sync hosts</button>",
      head + changes + published
    );
  }

  // -------------------------------------------------------------------- agents

  function post(path, body) {
    return request(path, { method: "POST", body: JSON.stringify(body || {}) });
  }

  function projectQuery(base) {
    if (!state.project || !state.project.path) return base;
    return base + (base.indexOf("?") === -1 ? "?" : "&") + "project=" + encodeURIComponent(state.project.path);
  }

  // ------------------------------------------------------------------- the team

  /* devd's own team: who is on it, what each member may do, what each must
   * prove, and which skills each carries. Read from team.roles, so this screen
   * cannot disagree with the capability the agents themselves use. */
  function loadRoster() {
    return Promise.all([
      read("team.roles"),
      state.projects.length ? Promise.resolve() : loadProjects()
    ]).then(function (r) {
      state.roster = r[0] || [];
      var path = state.rosterProject || (state.project && state.project.path) || (state.projects[0] && state.projects[0].path) || "";
      state.rosterProject = path;
      if (!path) return;
      return Promise.all([
        read("team.charter.get", { project: path }).catch(function () { return null; }),
        read("team.context", { project: path }).catch(function () { return null; })
      ]).then(function (data) {
        state.rosterCharter = data[0];
        state.rosterContext = data[1];
      });
    }).catch(function () {
      state.roster = state.roster || [];
      state.rosterCharter = state.rosterCharter || null;
      state.rosterContext = state.rosterContext || null;
    });
  }

  var TIER_WORD = {
    read: "reads only",
    write: "writes files",
    exec: "runs the project's checks",
    "admin (approval)": "installs with your approval"
  };

  var PROOF_WORD = { change: "files changed", verify: "a check passed", read: "the code was read", command: "a command ran" };

  function memberCard(r) {
    var proves = Object.keys(r.must_prove || {}).filter(function (k) { return r.must_prove[k]; }).map(function (k) { return PROOF_WORD[k]; });
    var detail = (proves.length ? "Counts as done only when: " + proves.join(" and ") + ". " : "") + ((r.skills || []).length ? "Skills: " + r.skills.join(", ") + "." : "");
    return uiRow({
      title: esc(r.title) + ' <span class="ui-quiet" style="display:inline;padding:0">@' + esc(r.id) + '</span>',
      sub: esc(r.summary), wrap: true,
      meta: esc(TIER_WORD[r.max_tier] || r.max_tier) + (r.works_in_own_copy ? " \u00b7 own copy" : ""),
      open: 'data-action="member-assign" data-id="' + attr(r.id) + '" title="' + attr(detail) + '"',
      actions: '<button type="button" class="btn btn-mini btn-primary" data-action="member-assign" data-id="' + attr(r.id) + '">Give work</button>',
      hover: true
    });
  }

  function roleTitle(id) {
    var r = (state.roster || []).filter(function (x) { return x.id === id; })[0];
    return r ? r.title : id;
  }

  function rolePills(ids) {
    if (!ids || !ids.length) return '<span class="dim">No roles invited yet.</span>';
    return ids.map(function (id) { return '<span class="pill pill-accent">' + esc(roleTitle(id)) + "</span>"; }).join(" ");
  }

  function teamContextHtml(ctx) {
    var sites = ctx.sites || [];
    var services = ctx.services || [];
    var runtimes = ctx.runtimes || [];
    var memory = ctx.memory || {};
    var lessons = ctx.lessons || [];
    var trust = ctx.trust || [];
    var siteHtml = sites.length
      ? sites.map(function (s) {
        return '<div class="context-line"><a href="' + attr(s.url) + '" target="_blank" rel="noreferrer">' + esc(s.url) + '</a> <span class="dim">' + esc(s.serving) + "</span></div>";
      }).join("")
      : '<div class="dim">No site is configured for this project.</div>';
    var serviceHtml = services.length
      ? '<div class="context-line">' + services.map(function (s) { return esc(s.name) + ' <span class="dim">' + esc(s.desired) + "</span>"; }).join(" · ") + "</div>"
      : '<div class="dim">No managed service.</div>';
    var runtimeHtml = runtimes.length
      ? '<div class="context-line">' + runtimes.map(function (r) { return esc(r.kind + " " + r.version); }).join(" · ") + "</div>"
      : '<div class="dim">No installed runtime.</div>';
    var memoryHtml = memory.present
      ? '<div class="context-line"><span class="mono">' + esc(memory.file) + "</span> <span class=\"dim\">" + (memory.truncated ? "read in part" : "read") + '</span></div><details class="ui-fold"><summary>Read the project knowledge</summary><pre class="log-block team-memory">' + esc(memory.excerpt) + "</pre></details>"
      : '<div class="dim">No AGENTS.md or CLAUDE.md in the project yet. The team has no written project knowledge.</div>';
    var lessonHtml = lessons.length
      ? '<ul class="team-lessons">' + lessons.slice(0, 5).map(function (l) { return "<li>" + esc(l) + "</li>"; }).join("") + "</ul>"
      : '<div class="dim">No lessons recorded yet. The first run starts the record.</div>';
    var trustHtml = trust.length
      ? trust.map(function (t) {
        return '<div class="trust-line"><span class="mono">' + esc(t.role) + '</span><span class="pill ' + (t.level === "proven" ? "pill-ok" : t.level === "building" ? "pill-accent" : "") + '">' + esc(t.level) + "</span><span class=\"dim\">" + t.done + " done" + (t.unverified || t.blocked_or_failed ? " · " + (t.unverified + t.blocked_or_failed) + " not accepted" : "") + "</span></div>";
      }).join("")
      : '<div class="dim">No completed task yet. Trust begins with the first verified run.</div>';
    return '<div class="team-context">' +
      '<div class="context-section"><div class="field-label">Sites</div>' + siteHtml + "</div>" +
      '<div class="context-section"><div class="field-label">Services</div>' + serviceHtml + "</div>" +
      '<div class="context-section"><div class="field-label">Runtimes</div>' + runtimeHtml + "</div>" +
      '<div class="context-section"><div class="field-label">Project knowledge</div>' + memoryHtml + "</div>" +
      '<div class="context-section"><div class="field-label">Trust record</div>' + trustHtml + "</div>" +
      '<div class="context-section"><div class="field-label">Lessons and missteps</div>' + lessonHtml + "</div>" +
      "</div>";
  }

  function teamRoomPanel() {
    var ctx = state.rosterContext || {};
    var charter = ctx.charter || state.rosterCharter || {};
    var north = charter.north_star || "";
    var owner = charter.owner || "";
    var rules = charter.working_agreement || [];
    var options = '<option value="">Choose a project…</option>' + projectOptions(state.rosterProject);
    return '<section class="panel team-room" aria-labelledby="team-room-title">' +
      '<div class="panel-head"><span id="team-room-title">Team room</span><span class="panel-note">the context every agent receives</span><span class="grow"></span>' +
        '<select class="select mono team-project" id="roster-project" aria-label="Project for the team room">' + options + "</select>" +
        '<button type="button" class="btn btn-mini" data-action="team-charter-edit">' + (north ? "Edit charter" : "Write the charter") + "</button></div>" +
      '<div class="panel-body team-room-body">' +
        '<div class="team-charter-block">' +
          '<div class="field-label">North star</div>' +
          (north ? '<div class="team-north">' + esc(north) + "</div>" : '<div class="team-north team-north-empty">No north star yet. Write what the team is trying to make true; it becomes the standard for every plan, question and review.</div>') +
          '<div class="team-meta"><span class="field-label">Human owner</span> ' + (owner ? esc(owner) : '<span class="dim">not named</span>') + "</div>" +
          '<div class="field-label" style="margin-top:14px">Working agreement</div>' +
          (rules.length ? '<ul class="team-rules">' + rules.map(function (r) { return "<li>" + esc(r) + "</li>"; }).join("") + "</ul>" : '<div class="dim">No agreement written.</div>') +
          '<div class="field-label" style="margin-top:14px">Invited to propose next work</div>' +
          '<div class="team-proactive">' + rolePills(charter.proactive_roles) + "</div>" +
        "</div>" +
        teamContextHtml(ctx) +
      "</div></section>";
  }

  function openCharterDrawer() {
    if (!state.rosterProject) return toast("warn", "Choose a project for the team room first");
    var ctx = state.rosterContext || {};
    var charter = ctx.charter || state.rosterCharter || {};
    var chosen = charter.proactive_roles || [];
    var roleBoxes = (state.roster || []).map(function (r) {
      return '<label class="check"><input type="checkbox" name="charter-role" value="' + attr(r.id) + '"' + (chosen.indexOf(r.id) !== -1 ? " checked" : "") + "> " + esc(r.title) + "</label>";
    }).join("");
    openDrawer("Team charter", '<div class="stack team-charter-form">' +
      '<div class="ui-quiet">' + esc(state.rosterProject) + "</div>" +
      '<div class="field"><label class="field-label" for="charter-owner">Human owner</label><input class="input" id="charter-owner" value="' + attr(charter.owner || "") + '" placeholder="Who owns the outcome and makes the final call?"></div>' +
      '<div class="field"><label class="field-label" for="charter-north">North star</label><textarea class="input" id="charter-north" rows="3" placeholder="One ambitious outcome the team can point every decision at.">' + esc(charter.north_star || "") + "</textarea></div>" +
      '<div class="field"><label class="field-label" for="charter-rules">Working agreement <span class="dim">(one rule per line)</span></label><textarea class="input" id="charter-rules" rows="5">' + esc((charter.working_agreement || []).join("\n")) + "</textarea></div>" +
      '<div class="field"><div class="field-label">Invite to propose next work</div><div class="check-grid">' + roleBoxes + "</div>" +
      '<div class="row-sub">These roles may suggest a next workstream that moves toward the north star. A person still approves the plan before anything runs.</div></div>' +
      '<div class="inline"><span class="grow"></span><button type="button" class="btn" data-action="team-charter-cancel">Cancel</button><button type="button" class="btn btn-primary" data-action="team-charter-save">Save charter</button></div>' +
      "</div>");
  }

  function renderRoster() {
    if (state.view !== "agents") return;
    var rows = state.roster || [];
    var groups = [];
    rows.forEach(function (r) {
      var g = groups.filter(function (x) { return x.key === r.team; })[0];
      if (!g) { g = { key: r.team, title: r.team_title || r.team, members: [] }; groups.push(g); }
      g.members.push(r);
    });
    var owner = state.rosterContext && state.rosterContext.charter ? state.rosterContext.charter.owner : "";
    var human = uiHead("Human", 1) + uiList([uiRow({
      dot: "ok",
      title: esc(owner || "You"),
      sub: "Owns the north star, the working agreement and the decisions that cannot be delegated.",
      meta: "human · final say"
    })]);
    var body = teamRoomPanel() + human + (groups.length
      ? uiHead("The roster", rows.length) + groups.map(function (g) { return uiHead(g.title, g.members.length) + uiList(g.members.map(memberCard)); }).join("")
      : uiEmpty("The team is not available", "team.roles did not answer."));
    el("main").innerHTML = page("Team", "The people, the written agreement, and the context every agent works from.",
      '<button type="button" class="btn btn-primary" data-action="nav" data-view="team">Give the team a goal</button>', body);
  }

  function openAssign(id) {
    var r = (state.roster || []).filter(function (x) { return x.id === id; })[0];
    if (!r) return;
    var choices = cwdChoices();
    var options = choices.map(function (c) { return '<option value="' + attr(c.path) + '">' + esc(c.label) + " - " + esc(c.path) + "</option>"; }).join("");
    openDrawer("Give work to " + r.title,
      '<div class="stack"><div class="row-sub">' + esc(r.summary) + "</div>" +
      '<div class="field"><label class="field-label" for="member-title">What to call it</label>' +
      '<input class="input" id="member-title" placeholder="Short title, optional"></div>' +
      '<div class="field"><label class="field-label" for="member-task">What they should do, and how you will know it worked</label>' +
      '<textarea class="input" id="member-task" rows="6" placeholder="Be concrete: the outcome, and the command or check that proves it."></textarea></div>' +
      '<div class="field"><label class="field-label" for="member-project">Project</label>' +
      '<select class="select mono" id="member-project">' + (options || '<option value="">No projects yet</option>') + "</select></div>" +
      '<div class="faint">They work in the project directory only. Anything above their permissions asks you first, and the task is accepted only when its tool results prove it.</div>' +
      '<button type="button" class="btn btn-primary" data-action="member-assign-submit" data-id="' + attr(r.id) + '">Start ' + esc(r.title) + "</button></div>");
  }

  function submitAssign(id) {
    var task = readInput("member-task");
    if (!task) return toast("warn", "Say what they should do");
    run("team.assign", {
      member: id,
      title: readInput("member-title") || null,
      instructions: task,
      project: readInput("member-project") || null
    }).then(function (outcome) {
      if (!outcome) return;
      closeDrawer();
      toast("ok", "The work has started. Follow it under Team runs.");
    });
  }

  // ----------------------------------------------------------------- processes

  /* The project folders a person can choose from. The projects devd knows
     come first; the projects folder itself is offered last so a new folder
     can be picked without registering it first. */
  function cwdChoices() {
    var out = [];
    (state.projects || []).forEach(function (p) {
      if (!p || !p.path) return;
      if (out.some(function (x) { return x.path === p.path; })) return;
      out.push({ label: p.name || dirName(p.path), path: p.path });
    });
    var www = state.session && state.session.paths ? state.session.paths.www : null;
    if (www && !out.some(function (x) { return x.path === www; })) {
      out.push({ label: "devd projects folder", path: www });
    }
    return out;
  }

  function processProject() {
    if (state.processes.project) return state.processes.project;
    if (state.project && state.project.path) return state.project.path;
    return state.projects[0] ? state.projects[0].path : (state.session ? state.session.paths.www : null);
  }

  function projectOptions(selected) {
    return cwdChoices().map(function (c) {
      return '<option value="' + attr(c.path) + '"' + (c.path === selected ? " selected" : "") + ">" +
        esc(c.label) + " — " + esc(c.path) + "</option>";
    }).join("");
  }

  function loadProcesses() {
    var project = processProject();
    state.processes.project = project;
    if (!project) return Promise.resolve();
    state.processes.loading = true;
    return request("/api/project?project=" + encodeURIComponent(project)).then(function (res) {
      state.processes.loading = false;
      if (!res.ok) {
        state.processes.plan = null;
        state.processes.status = null;
        state.processes.error = errorOf(res.body, "no devd.yml in this project");
        return;
      }
      state.processes.error = null;
      state.processes.plan = res.body;
      return request("/api/project/status?project=" + encodeURIComponent(project)).then(function (status) {
        state.processes.status = status.ok ? status.body : null;
        if (!status.ok) state.processes.error = errorOf(status.body, "could not read process status");
      });
    }).catch(function (e) {
      state.processes.loading = false;
      state.processes.error = e.message;
    });
  }

  function renderProcesses() {
    if (state.view !== "processes") return;
    var p = state.processes;
    var project = processProject();
    p.project = project;
    if (project && !p.plan && !p.error && !p.loading) {
      p.loading = true;
      loadProcesses().then(renderProcesses);
    }
    var select = '<div class="ui-controls"><label class="field-label" for="process-project" style="margin:0">Project</label>' +
      '<select class="select mono" id="process-project" style="max-width:420px">' + (projectOptions(project) || '<option value="">No projects</option>') + "</select></div>";

    if (p.loading && !p.plan) {
      el("main").innerHTML = page("Processes", "", "", select + '<div class="loading" role="status">Reading devd.yml…</div>');
      return;
    }

    if (!project || (p.error && !p.plan)) {
      // A file that is there but invalid is not the same as no file: say which.
      var missing = !p.error || /no devd\.yml|not found|cannot find|does not exist/i.test(p.error);
      el("main").innerHTML = page("Processes", "", "", select +
        (!missing ? '<div class="notice notice-bad" role="alert"><b>devd.yml could not be used.</b> ' + esc(p.error) + ". Fix the file; nothing in it runs until it is valid and approved.</div>" : "") +
        uiEmpty(missing ? "No devd.yml in this project" : "No processes to show", (missing && p.error ? p.error + ". " : "") +
          "A project lists the long-running commands it needs (a dev server, a queue worker, a watcher) in a devd.yml at its root. devd shows you the exact commands and runs nothing until you approve them.") +
          (missing ? '<div class="uiHead-spacer"></div>' + uiHead("What it looks like", null, "") + '<pre class="log-block">version: 1\nprocesses:\n  web:\n    command: ["npm", "run", "dev"]\n    health:\n      url: "http://127.0.0.1:3000/"\n  worker:\n    command: ["npm", "run", "worker"]\n    depends_on: ["web"]</pre>' : ""));
      return;
    }

    var plan = p.plan || { processes: [] };
    var status = (p.status && p.status.processes) || [];
    var byName = {};
    status.forEach(function (row) { byName[row.name] = row; });

    // The manifest's commands run only once a person has approved them, exactly as written.
    var approval = plan.approved
      ? '<div class="ui-more" style="margin-top:0"><span class="fact fact-ok">commands approved</span> <span class="mono">' + esc(String(plan.hash || "").slice(0, 12)) +
        "</span> - editing any command in devd.yml withdraws the approval.</div>"
      : '<div class="notice notice-warn" role="status"><b>Not approved.</b> Nothing in this devd.yml runs until you have read the commands below and approved them exactly as written. ' +
        '<button type="button" class="btn btn-primary btn-mini" data-action="approve-project">Approve these commands</button></div>';

    var counts = { running: 0, failed: 0 };
    var rows = (plan.processes || []).map(function (spec) {
      var row = byName[spec.name] || {};
      var stateName = row.status || "stopped";
      if (stateName === "running") counts.running++;
      if (stateName === "failed") counts.failed++;
      var health = row.health || (spec.health_declared ? "unknown" : "none");
      var live = stateName === "running" || stateName === "starting";
      var facts = [
        stateName === "running" ? '<span class="fact fact-ok">running</span>' : stateName === "starting" ? '<span class="fact">' + uiDot("live") + "starting</span>" :
          stateName === "failed" ? '<span class="fact fact-bad">failed</span>' : '<span class="fact">' + esc(stateName) + "</span>"
      ];
      // Health is its own fact: a running process is not a healthy one until its check passes.
      if (live) facts.push(health === "healthy" ? '<span class="fact fact-ok">healthy</span>' : health === "none" ? '<span class="fact" title="devd.yml declares no health check for it">no health check</span>' :
        health === "unhealthy" ? '<span class="fact fact-bad">unhealthy</span>' : '<span class="fact fact-warn">health ' + esc(health) + "</span>");
      if (row.pid) facts.push('<span class="fact mono">pid ' + esc(row.pid) + "</span>");
      if ((spec.depends_on || []).length) facts.push('<span class="fact">after ' + esc(spec.depends_on.join(", ")) + "</span>");
      var actions = live
        ? '<button type="button" class="btn btn-mini" data-action="process-stop" data-name="' + attr(spec.name) + '">Stop</button>' +
          '<button type="button" class="btn btn-mini btn-ghost" data-action="process-restart" data-name="' + attr(spec.name) + '">Restart</button>'
        : '<button type="button" class="btn btn-mini btn-primary" data-action="process-start" data-name="' + attr(spec.name) + '"' +
          (plan.approved ? "" : ' disabled title="Approve the commands first"') + ">Start</button>";
      actions += '<button type="button" class="btn btn-mini btn-ghost" data-action="process-logs" data-name="' + attr(spec.name) + '">Logs</button>';
      return uiRow({
        dot: stateName === "failed" || health === "unhealthy" ? "bad" : live ? (health === "healthy" || health === "none" ? "ok" : "warn") : "off",
        title: '<span class="row-name">' + esc(spec.name) + '</span> <span class="facts">' + facts.join("") + "</span>",
        sub: '<span class="mono">' + esc((spec.command || []).join(" ")) + "</span>" +
          (row.last_error ? '<div class="ui-row-err">' + esc(row.last_error) + "</div>" : ""),
        wrap: true,
        actions: actions
      });
    });
    var errorRow = "";

    var log = "";
    if (p.selected) {
      var lines = (p.log && p.log.lines) || [];
      var q = (p.logQuery || "").toLowerCase();
      if (q) lines = lines.filter(function (l) { return l.toLowerCase().indexOf(q) !== -1; });
      log = '<div class="panel process-log"><div class="panel-head"><span>Logs - <span class="mono">' + esc(p.selected) + "</span></span>" +
        '<input class="input mono process-log-filter" id="process-log-query" value="' + attr(p.logQuery || "") + '" placeholder="Filter lines" aria-label="Filter log lines">' +
        '<button type="button" class="btn btn-mini" data-action="close-process-logs">Close</button></div>' +
        '<div class="panel-body flush"><pre class="log-pre" id="process-log">' +
        (lines.length ? esc(lines.join("\n")) : "No output yet.") + "</pre></div></div>";
    }

    el("main").innerHTML = page(
      "Processes",
      "Supervised processes declared in devd.yml, with real pids, logs and health.",
      '<button type="button" class="btn" data-action="process-start-all"' + (plan.approved ? "" : " disabled") + ">Start all</button>" +
        '<button type="button" class="btn" data-action="process-stop-all">Stop all</button>' +
        '<button type="button" class="btn btn-ghost" data-action="process-refresh">Refresh</button>',
      select + approval +
        uiHead(plan.project || "Processes", (plan.processes || []).length,
          '<span class="ui-quiet mono" style="padding:0">' + esc(plan.root || "") + "</span>") +
        (rows.length ? uiList(rows) : uiEmpty("No processes declared", "devd.yml lists none.")) +
        (counts.failed ? '<div class="ui-more">' + counts.failed + (counts.failed === 1 ? " process has" : " processes have") + " stopped with an error; its last output is in Logs.</div>" : "") +
        errorRow + log
    );
  }

  function processAction(kind, name, all) {
    var project = state.processes.project;
    if (!project) return toast("warn", "Choose a project first");
    if (!state.processes.plan || !state.processes.plan.approved) {
      return toast("warn", "Review and approve the manifest commands first");
    }
    state.processes.busy = true;
    var body = { project: project, name: name || "", all: !!all };
    post("/api/project/" + kind, body).then(function (res) {
      state.processes.busy = false;
      if (!res.ok) {
        toast("error", errorOf(res.body, "the process operation failed"));
      } else if (res.body.outcome && res.body.outcome.warnings && res.body.outcome.warnings.length) {
        toast("warn", res.body.outcome.warnings.join("; "));
      } else {
        toast("ok", "process " + kind + " applied");
      }
      loadProcesses().then(renderProcesses);
    });
  }

  function approveProject() {
    var project = state.processes.project;
    if (!project) return;
    post("/api/project/approve", { project: project }).then(function (res) {
      if (!res.ok) return toast("error", errorOf(res.body, "could not approve the manifest"));
      toast("ok", "The exact commands in devd.yml are approved");
      loadProcesses().then(renderProcesses);
    });
  }

  function openProcessLogs(name) {
    state.processes.selected = name;
    state.processes.logQuery = "";
    loadProcessLogs().then(renderProcesses);
    startProcessTimer();
  }

  function loadProcessLogs() {
    var p = state.processes;
    if (!p.selected || !p.project) return Promise.resolve();
    return request("/api/project/logs?project=" + encodeURIComponent(p.project) +
      "&name=" + encodeURIComponent(p.selected) + "&lines=400").then(function (res) {
      if (res.ok) p.log = res.body;
    });
  }

  function startProcessTimer() {
    if (state.processes.timer) return;
    state.processes.timer = setInterval(function () {
      if (state.view !== "processes") {
        clearInterval(state.processes.timer);
        state.processes.timer = null;
        return;
      }
      if (document.activeElement && document.activeElement.id === "process-log-query") return;
      loadProcesses().then(function () {
        if (state.processes.selected) {
          return loadProcessLogs().then(renderProcesses);
        }
        renderProcesses();
      });
    }, 2200);
  }

  // -------------------------------------------------------------------- shared

  function sharedProject() {
    if (state.shared.project) return state.shared.project;
    if (state.project && state.project.path) return state.project.path;
    return state.projects[0] ? state.projects[0].path : (state.session ? state.session.paths.www : null);
  }

  function loadShared() {
    var project = sharedProject();
    state.shared.project = project;
    if (!project) return Promise.resolve();
    state.shared.loading = true;
    return Promise.all([
      read("scratch.list", { project: project }).catch(function () { return []; }),
      read("todo.list", { project: project }).catch(function () { return []; })
    ]).then(function (r) {
      state.shared.loading = false;
      state.shared.notes = Array.isArray(r[0]) ? r[0] : [];
      state.shared.todos = Array.isArray(r[1]) ? r[1] : [];
      if (state.shared.activeNote) {
        var found = state.shared.notes.filter(function (n) { return n.name === state.shared.activeNote; })[0];
        if (!found) {
          state.shared.activeNote = null;
          state.shared.noteContent = "";
        }
      }
    });
  }

  // ------------------------------------------------------------------ skills

  /* The skills the team can load.
   *
   * A skill is a short set of instructions an agent reads when the work
   * calls for it. They are installed from a GitHub repository - where the
   * skills published on skills.sh live - or imported from another agent that
   * already has them. */
  var SKILLS = { list: null, sources: null, tab: "installed" };
  var MCP = { servers: null, error: "" };
  var STUDIO = { tab: "review", project: "", reviews: null, selected: null, memory: null, memoryOpen: null, compat: null };

  var SCHED = { list: null, templates: [], form: { name: "", spec: "weekdays 09:30", project: "", template: "", params: "", goal: "", done: "", waive: false } };

  function loadSchedules() {
    return Promise.all([
      read("schedule.list").catch(function () { return []; }),
      read("template.list").catch(function () { return []; }),
      state.projects.length ? Promise.resolve() : loadProjects()
    ]).then(function (r) {
      SCHED.list = Array.isArray(r[0]) ? r[0] : [];
      SCHED.templates = Array.isArray(r[1]) ? r[1] : [];
    });
  }

  function whenText(iso) {
    if (!iso) return "never";
    var d = new Date(iso);
    return isNaN(d.getTime()) ? iso : d.toLocaleString();
  }

  function renderSchedules() {
    if (state.view !== "schedules") return;
    if (!SCHED.list) return renderLoading();
    var f = SCHED.form;
    var rows = SCHED.list.map(function (sc) {
      var what = sc.template ? "template " + sc.template : "a goal";
      return "<tr>" +
        '<td><b>' + esc(sc.name) + "</b><div class=\"dim\">" + esc(sc.spec) + (sc.auto_approve ? " · plan approval waived" : "") + "</div></td>" +
        "<td>" + esc(what) + '<div class="dim mono">' + esc(sc.project) + "</div></td>" +
        "<td>" + (sc.enabled ? esc(whenText(sc.next_run_at)) : '<span class="dim">off</span>') + "</td>" +
        "<td>" + (sc.last_run_at ? esc(whenText(sc.last_run_at)) : '<span class="dim">not yet</span>') +
          (sc.last_error ? '<div class="dim">' + esc(sc.last_error) + "</div>" : "") +
          (sc.last_run_id ? '<div><button type="button" class="btn btn-mini btn-ghost" data-action="team-open-from" data-id="' + attr(sc.last_run_id) + '">Open the run</button></div>' : "") + "</td>" +
        '<td class="actions"><button type="button" class="btn btn-mini" data-action="schedule-run" data-name="' + attr(sc.name) + '">Run now</button> ' +
        '<button type="button" class="btn btn-mini" data-action="schedule-toggle" data-name="' + attr(sc.name) + '" data-on="' + (sc.enabled ? "1" : "0") + '">' + (sc.enabled ? "Turn off" : "Turn on") + "</button> " +
        '<button type="button" class="btn btn-mini btn-ghost" data-action="schedule-delete" data-name="' + attr(sc.name) + '">Delete</button></td></tr>';
    }).join("");
    var tpl = SCHED.templates.map(function (t) { return '<option value="' + attr(t.id) + '"' + (f.template === t.id ? " selected" : "") + ">" + esc(t.title) + "</option>"; }).join("");
    var chosen = SCHED.templates.filter(function (t) { return t.id === f.template; })[0];
    var hint = chosen ? (chosen.params || []).map(function (p) { return p.name + "=" + (p.help || "") + (p.optional ? " (optional)" : ""); }).join("\n") : "";
    var projects = '<option value="">Choose a project…</option>' + state.projects.map(function (p) { return '<option value="' + attr(p.path) + '"' + (f.project === p.path ? " selected" : "") + ">" + esc(p.name || dirName(p.path)) + "</option>"; }).join("");
    el("main").innerHTML = page("Schedules", "Team runs devd starts on a clock", "",
      '<div class="panel"><div class="panel-head"><span>Schedules</span><span class="panel-note">they run only while devd is open or serving</span></div>' +
      (SCHED.list.length ? '<div class="table-wrap"><table class="table"><thead><tr><th>Name</th><th>What</th><th>Next</th><th>Last</th><th></th></tr></thead><tbody>' + rows + "</tbody></table></div>"
        : '<div class="panel-body empty-inline">No schedules yet. A schedule starts the team on a project at times you choose; it never piles runs up and a person still approves each plan unless you waive it.</div>') + "</div>" +
      '<div class="panel"><div class="panel-head"><span>New schedule</span></div><div class="panel-body stack">' +
      '<div class="inline"><input class="input" id="sched-name" placeholder="name, e.g. weekly-security" value="' + attr(f.name) + '" style="max-width:240px" spellcheck="false">' +
      '<input class="input" id="sched-spec" placeholder="weekdays 09:30" value="' + attr(f.spec) + '" style="max-width:200px" spellcheck="false" title="every 30m, every 6h, daily 09:30, weekdays 09:30, mon,wed 18:00 (local time)">' +
      '<select class="input" id="sched-project" style="max-width:280px">' + projects + "</select></div>" +
      '<div class="inline"><select class="input" id="sched-template" style="max-width:320px"><option value="">A goal I write</option>' + tpl + "</select></div>" +
      (chosen ? '<div class="field"><label class="field-label">Values <span class="dim">(name=value, one per line)</span></label><textarea class="input mono" id="sched-params" rows="3" placeholder="' + attr(hint) + '">' + esc(f.params) + "</textarea></div>"
        : '<div class="field"><label class="field-label">Goal</label><textarea class="input" id="sched-goal" rows="3" placeholder="What the team should do each time">' + esc(f.goal) + "</textarea></div>") +
      '<div class="field"><label class="field-label">More to prove <span class="dim">(optional, one per line)</span></label><textarea class="input" id="sched-done" rows="4">' + esc(f.done) + "</textarea></div>" +
      '<label class="check"><input type="checkbox" id="sched-waive"' + (f.waive ? " checked" : "") + '> Start each run without asking me to approve the plan <span class="dim">(not in production mode)</span></label>' +
      '<div><button type="button" class="btn btn-primary" data-action="schedule-create">Create the schedule</button></div></div></div>');
  }

  function loadSkills() {
    return Promise.all([
      read("skills.list").catch(function () { return []; }),
      read("skills.sources").catch(function () { return []; })
    ]).then(function (r) {
      SKILLS.list = Array.isArray(r[0]) ? r[0] : [];
      SKILLS.sources = Array.isArray(r[1]) ? r[1] : [];
    });
  }

  function renderSkills() {
    if (state.view !== "skills") return;
    var list = SKILLS.list || [];
    var sources = SKILLS.sources || [];
    var q = state.filters.skills || "";

    var kinds = { playbook: "Playbooks", technique: "Techniques", principle: "Principles" };
    var groups = {};
    list
      .filter(function (s) { return keeps(q, s.name, s.description, s.kind, (s.roles || []).join(" ")); })
      .forEach(function (s) { (groups[s.kind || "technique"] = groups[s.kind || "technique"] || []).push(s); });

    var body = '<div class="filter-row">' + filterBox("skills", "Search skills") + "</div>";

    if (!list.length) {
      body += '<div class="panel"><div class="empty"><div class="empty-title">No skills</div>' +
        "<div>devd ships a small set of its own. Install more from a GitHub repository, or import the ones another agent on this machine already has.</div></div></div>";
    } else {
      Object.keys(kinds).forEach(function (k) {
        var items = groups[k];
        if (!items || !items.length) return;
        body += '<div class="panel"><div class="panel-head"><span>' + kinds[k] + "</span>" +
          '<span class="panel-note">' + items.length + "</span></div><div class=\"rows\">" +
          items.map(function (s) {
            return '<div class="svc"><div class="svc-main"><div class="svc-name">' +
              esc(s.name) + ' <span class="pill">' + esc(s.source) + "</span>" +
              (s.roles && s.roles.length ? ' <span class="dim mono">' + esc(s.roles.join(", ")) + "</span>" : "") +
              '</div><div class="svc-sub">' + esc(s.description) + "</div></div>" +
              '<div class="svc-actions">' +
              '<button type="button" class="btn btn-mini" data-action="skill-read" data-name="' + attr(s.name) + '">Read</button>' +
              (s.source === "user" ? '<button type="button" class="btn btn-mini btn-danger" data-action="skill-remove" data-name="' + attr(s.name) + '">Remove</button>' : "") +
              "</div></div>";
          }).join("") + "</div></div>";
      });
      if (q && !Object.keys(groups).length) {
        body += '<div class="panel"><div class="panel-body empty-inline">No skill matches "' + esc(q) + '".</div></div>';
      }
    }

    body += '<div class="panel"><div class="panel-head"><span>Install</span>' +
      '<span class="panel-note">from GitHub, where the skills on skills.sh live</span></div>' +
      '<div class="panel-body stack">' +
      '<div class="inline"><input class="input mono grow" id="skill-source" placeholder="owner/name, for example anthropics/skills" spellcheck="false">' +
      '<button type="button" class="btn btn-primary" data-action="skill-install">Install</button></div>' +
      '<div class="row-sub">A skill is a folder with a SKILL.md in it. devd reads the repository, fetches the skill files as text and saves them in its own folder. Nothing is executed. ' +
      '<button type="button" class="btn btn-mini btn-ghost" data-action="skill-browse">Browse the catalogue</button></div>' +
      "</div></div>";

    body += '<div class="panel"><div class="panel-head"><span>Import</span>' +
      '<span class="panel-note">skills another agent on this machine already has</span></div>' +
      '<div class="panel-body flush">' +
      (sources.length
        ? "<table class=\"table\"><thead><tr><th>Agent</th><th>Skills</th><th>Folder</th><th></th></tr></thead><tbody>" +
          sources.map(function (s) {
            return '<tr><td class="cell-strong">' + esc(s.agent) + '</td><td class="cell-mono">' + s.count + "</td>" +
              '<td class="cell-mono cell-dim" title="' + attr(s.path) + '">' + esc(s.path) + "</td>" +
              '<td class="cell-actions"><button type="button" class="btn btn-mini" data-action="skill-import" data-agent="' + attr(s.agent) + '">Import</button></td></tr>';
          }).join("") + "</tbody></table>"
        : '<div class="panel-body empty-inline">No other agent on this machine keeps a folder of skills.</div>') +
      "</div></div>";

    el("main").innerHTML = page(
      "Skills",
      "Playbooks, techniques and principles the team loads when the work calls for them.",
      "",
      body
    );
  }

  function renderShared() {
    if (state.view !== "shared") return;    var s = state.shared;
    var project = sharedProject();
    s.project = project;
    var tabs = '<div class="segmented">' +
      '<button type="button" class="seg' + (s.tab === "todos" ? " active" : "") + '" data-action="shared-tab" data-tab="todos">Todos <span class="rail-count">' + s.todos.filter(function (t) { return t.status !== "done" && t.status !== "cancelled"; }).length + "</span></button>" +
      '<button type="button" class="seg' + (s.tab === "notes" ? " active" : "") + '" data-action="shared-tab" data-tab="notes">Scratchpads <span class="rail-count">' + s.notes.length + "</span></button></div>";

    var head = '<div class="panel"><div class="panel-head"><span>Shared project state</span>' +
      '<select class="select mono" id="shared-project">' + (projectOptions(project) || '<option value="">No projects</option>') + "</select></div>" +
      '<div class="panel-body row-sub">Notes and todos are project-scoped, durable, and readable by agents through the same capability API. Reloading devd does not lose them.</div></div>';

    var body;
    if (s.tab === "todos") {
      var create = s.todoOpen
        ? '<div class="panel"><div class="panel-body stack">' +
          '<div class="inline"><input class="input grow" id="todo-title" placeholder="What needs doing?">' +
          '<select class="select" id="todo-priority"><option value="normal">normal</option><option value="high">high</option><option value="urgent">urgent</option><option value="low">low</option></select></div>' +
          '<input class="input" id="todo-assignee" placeholder="Assignee (optional, e.g. agent:opencode)">' +
          '<div class="inline"><button type="button" class="btn btn-primary" data-action="todo-create">Create todo</button>' +
          '<button type="button" class="btn btn-ghost" data-action="todo-cancel">Cancel</button></div></div></div>'
        : "";
      var rows = s.todos.length ? s.todos.map(function (t) {
        var open = t.status !== "done" && t.status !== "cancelled";
        return '<div class="todo-row"><div class="todo-main">' +
          '<div class="todo-title">' + esc(t.title) + ' <span class="pill ' + (t.status === "done" ? "pill-ok" : t.status === "blocked" ? "pill-warn" : "") + '">' + esc(t.status) + "</span>" +
          ' <span class="pill">' + esc(t.priority) + "</span></div>" +
          (t.description ? '<div class="todo-desc">' + esc(t.description) + "</div>" : "") +
          '<div class="todo-meta mono">' + esc(t.assignee || "unassigned") + " · rev " + esc(t.revision) + " · " + esc(t.updated_at || "") + "</div></div>" +
          '<div class="inline">' +
          '<select class="select compact" data-action="todo-status" data-id="' + attr(t.id) + '" data-rev="' + attr(t.revision) + '">' +
          ["open", "in_progress", "blocked", "done", "cancelled"].map(function (st) {
            return '<option value="' + st + '"' + (t.status === st ? " selected" : "") + ">" + st + "</option>";
          }).join("") + "</select>" +
          (open ? '<button type="button" class="btn btn-mini" data-action="todo-done" data-id="' + attr(t.id) + '" data-rev="' + attr(t.revision) + '">Done</button>'
                : '<button type="button" class="btn btn-mini" data-action="todo-reopen" data-id="' + attr(t.id) + '" data-rev="' + attr(t.revision) + '">Reopen</button>') +
          '<button type="button" class="btn btn-mini btn-ghost" data-action="todo-delete" data-id="' + attr(t.id) + '">Delete</button>' +
          "</div></div>";
      }).join("") : '<div class="empty-inline">No todos yet.</div>';
      body = '<div class="panel"><div class="panel-head"><span>Todos</span>' +
        '<button type="button" class="btn btn-mini btn-primary" data-action="todo-open">New todo</button></div>' +
        '<div class="panel-body flush">' + create + rows + "</div></div>";
    } else {
      var noteList = s.notes.length ? s.notes.map(function (n) {
        return '<button type="button" class="note-row' + (s.activeNote === n.name ? " active" : "") + '" data-action="note-open" data-name="' + attr(n.name) + '">' +
          '<span class="cell-strong">' + esc(n.name) + "</span>" +
          '<span class="cell-mono">rev ' + esc(n.revision) + " · " + esc(n.author) + "</span></button>";
      }).join("") : '<div class="empty-inline">No scratchpads yet.</div>';
      var editor = s.activeNote
        ? '<div class="note-editor"><div class="note-editor-head"><span class="cell-strong">' + esc(s.activeNote) + "</span>" +
          '<span class="cell-mono">revision ' + esc(s.noteRevision) + "</span></div>" +
          '<textarea class="input mono note-text" id="note-content" rows="16">' + esc(s.noteContent) + "</textarea>" +
          '<div class="inline"><button type="button" class="btn btn-primary" data-action="note-save">Save</button>' +
          '<button type="button" class="btn btn-ghost" data-action="note-delete">Delete note</button></div></div>'
        : '<div class="empty-inline">Select a note, or create one. Agents use scratch.append to add findings without replacing yours.</div>';
      var create = s.noteOpen
        ? '<div class="inline" style="padding:10px"><input class="input mono grow" id="note-name" placeholder="note-name">' +
          '<button type="button" class="btn btn-primary" data-action="note-create">Create</button>' +
          '<button type="button" class="btn btn-ghost" data-action="note-cancel">Cancel</button></div>'
        : "";
      body = '<div class="note-layout"><div class="panel note-list-panel"><div class="panel-head"><span>Scratchpads</span>' +
        '<button type="button" class="btn btn-mini" data-action="note-open-create">New note</button></div>' +
        '<div class="panel-body flush">' + create + noteList + "</div></div>" +
        '<div class="panel"><div class="panel-body">' + editor + "</div></div></div>";
    }

    el("main").innerHTML = page(
      "Shared",
      "Project notes and todos for humans and agents — durable across sessions and reloads.",
      tabs,
      head + body
    );
  }

  function createTodo() {
    var title = readInput("todo-title");
    if (!title) return toast("warn", "A todo needs a title");
    run("todo.create", {
      project: sharedProject(),
      title: title,
      priority: readInput("todo-priority") || "normal",
      assignee: readInput("todo-assignee") || null
    }).then(function (outcome) {
      if (!outcome) return;
      state.shared.todoOpen = false;
      loadShared().then(renderShared);
    });
  }

  function setTodoStatus(id, rev, status) {
    run("todo.update", { id: id, expected_revision: Number(rev), status: status, note: "status changed in the window" })
      .then(function (outcome) {
        if (!outcome) return loadShared().then(renderShared);
        loadShared().then(renderShared);
      });
  }

  function completeTodo(id, rev) {
    run("todo.complete", { id: id, expected_revision: Number(rev) }).then(function (outcome) {
      if (outcome) loadShared().then(renderShared);
    });
  }

  function reopenTodo(id, rev) {
    run("todo.reopen", { id: id, expected_revision: Number(rev) }).then(function (outcome) {
      if (outcome) loadShared().then(renderShared);
    });
  }

  function deleteTodo(id) {
    run("todo.delete", { id: id }).then(function (outcome) {
      if (outcome) loadShared().then(renderShared);
    });
  }

  function openNote(name) {
    read("scratch.get", { project: sharedProject(), name: name }).then(function (note) {
      if (!note) return;
      state.shared.activeNote = note.name;
      state.shared.noteContent = note.content;
      state.shared.noteRevision = note.revision;
      renderShared();
    }).catch(function (e) { toast("error", e.message); });
  }

  function saveNote() {
    var content = el("note-content") ? el("note-content").value : "";
    run("scratch.update", {
      project: sharedProject(),
      name: state.shared.activeNote,
      content: content,
      expected_revision: state.shared.noteRevision
    }).then(function (outcome) {
      if (!outcome) return loadShared().then(renderShared);
      loadShared().then(function () {
        return openNote(state.shared.activeNote);
      });
    });
  }

  function createNote() {
    var name = readInput("note-name");
    if (!name) return toast("warn", "A note needs a name");
    run("scratch.create", { project: sharedProject(), name: name, content: "" }).then(function (outcome) {
      if (!outcome) return;
      state.shared.noteOpen = false;
      state.shared.activeNote = name;
      state.shared.noteContent = "";
      state.shared.noteRevision = 1;
      loadShared().then(renderShared);
    });
  }

  function deleteNote() {
    run("scratch.delete", { project: sharedProject(), name: state.shared.activeNote }).then(function (outcome) {
      if (!outcome) return;
      state.shared.activeNote = null;
      state.shared.noteContent = "";
      loadShared().then(renderShared);
    });
  }

  // ------------------------------------------------------------------- preview

  function renderPreview() {
    if (state.view !== "preview") return;
    var sites = state.server.sites;
    if (!sites.length) {
      el("main").innerHTML = page(
        "Preview",
        "The site you are working on, inside the window.",
        "",
        '<div class="panel"><div class="empty"><div class="empty-title">No sites to preview</div>' +
          "<div>Create a site and it appears here.</div>" +
          '<div class="inline" style="margin-top:12px"><button type="button" class="btn btn-primary" data-action="nav" data-view="sites">Create a site</button></div>' +
          "</div></div>"
      );
      return;
    }

    var name = state.preview.site;
    var row =
      sites.filter(function (s) { return s.site.name === name; })[0] || sites[0];
    state.preview.site = row.site.name;

    var options = sites
      .map(function (s) {
        return (
          '<option value="' + attr(s.site.name) + '"' + (s.site.name === row.site.name ? " selected" : "") + ">" +
          esc(s.site.name) + " — " + esc(s.url) + "</option>"
        );
      })
      .join("");

    var notice = caTrusted()
      ? ""
      : '<div class="notice" style="margin-bottom:12px"><div class="grow">The certificate authority is not trusted yet, so an HTTPS preview may show a warning.</div>' +
        '<button type="button" class="btn btn-mini" data-action="ca-trust" data-machine="false">Trust the CA</button></div>';

    el("main").innerHTML = page(
      "Preview",
      "The running site, inside the window.",
      '<button type="button" class="btn" data-action="reload-preview">Reload</button>' +
        '<button type="button" class="btn" data-action="open-url" data-url="' + attr(row.url) + '">Open externally</button>',
      notice +
        '<div class="panel"><div class="panel-head">' +
        '<select class="select" id="preview-site">' + options + "</select>" +
        '<span class="panel-note mono">' + esc(row.url) + "</span></div>" +
        '<div class="panel-body flush"><iframe id="preview-frame" class="preview-frame" sandbox="allow-scripts allow-same-origin allow-forms allow-popups allow-downloads allow-modals" src="' +
        attr(row.url) + '" title="' + attr(row.site.name) + '"></iframe></div></div>'
    );
  }

  /*
   * Register a folder as a project.
   *
   * The folder is not moved, not copied and not changed: devd records where it
   * is and what is in it. A folder that is also a Git repository is indexed as
   * one, so that the Git screens and the project screens talk about the same
   * thing.
   */
  function addProjectAt(path) {
    return run("project.import", { path: path })
      .then(function (outcome) {
        if (!outcome) return null;
        var data = outcome.data || {};
        var isRepo = !!(data.git && data.git.repository);
        if (!isRepo) return outcome;
        return run("git.repo.track", { repo: path }).then(function () { return outcome; });
      })
      .then(function (outcome) {
        if (!outcome) return null;
        return loadProjects().then(function () {
          navigate("projects");
          return outcome;
        });
      });
  }

  // ------------------------------------------------------------------ projects
  //
  // A project is a folder on the person's disk. devd does not create them and
  // does not own them: it knows where they are, what is in them, and what
  // state they are in. Adding one is therefore a matter of *choosing a
  // folder*, which is what the picker is for.

  var PROJECT_SORTS = [
    { id: "name", label: "Name" },
    { id: "recent", label: "Recently touched" },
    { id: "status", label: "Needs attention" }
  ];

  function projectSortLabel() {
    for (var i = 0; i < PROJECT_SORTS.length; i++) {
      if (PROJECT_SORTS[i].id === state.projectSort) return PROJECT_SORTS[i].label;
    }
    return "Name";
  }

  /* Pinned projects, kept per browser because pinning is a personal view
     preference rather than a fact about the project. */
  function pinnedProjects() {
    try {
      var raw = window.localStorage.getItem("devd.pinned");
      var list = raw ? JSON.parse(raw) : [];
      return Array.isArray(list) ? list : [];
    } catch (e) {
      return [];
    }
  }

  function togglePin(path) {
    var list = pinnedProjects();
    var at = list.indexOf(path);
    if (at === -1) list.push(path);
    else list.splice(at, 1);
    try {
      window.localStorage.setItem("devd.pinned", JSON.stringify(list));
    } catch (e) {
      // A browser that refuses storage still works; the pin just does not stick.
    }
    renderProjects();
  }

  function projectChanges(st) {
    if (!st) return 0;
    return (st.staged || []).length + (st.unstaged || []).length + (st.untracked || []).length;
  }

  /* Git refuses a repository owned by another account. That is a real,
     actionable condition, not a reason to pretend the project has no changes. */
  function gitErrorShort(message) {
    var m = String(message || "");
    if (/dubious ownership/i.test(m)) return "Git cannot read it: owned by another user";
    if (/not a Git repository/i.test(m)) return "not a Git repository";
    if (/could not|failed|error/i.test(m)) return "Git could not read it";
    return "";
  }

  function renderProjects() {
    if (state.view !== "projects") return;
    var query = state.projectQuery.trim().toLowerCase();
    var pinned = pinnedProjects();
    var all = state.projects.slice();
    var siteFor = {};
    (state.server.sites || []).forEach(function (row) {
      if (row.project && !siteFor[row.project]) siteFor[row.project] = row;
    });
    var visible = all.filter(function (p) {
      if (!query) return true;
      var site = siteFor[p.path];
      return keeps(query, p.name, p.path, p.stack, site && site.site && site.site.name, site && servingLabel(site));
    });
    visible.sort(function (a, b) {
      var ap = pinned.indexOf(a.path) !== -1 ? 0 : 1;
      var bp = pinned.indexOf(b.path) !== -1 ? 0 : 1;
      if (ap !== bp) return ap - bp;
      if (state.projectSort === "recent") return String(b.updated_at || "").localeCompare(String(a.updated_at || ""));
      if (state.projectSort === "status") {
        var ac = projectChanges(a.status), bc = projectChanges(b.status);
        if (ac !== bc) return bc - ac;
      }
      return String(a.name || a.path).localeCompare(String(b.name || b.path));
    });

    var body;
    if (!state.projectsLoaded) {
      body = '<div class="loading" role="status" aria-live="polite">Reading your projects…</div>';
    } else if (state.projectsError && !all.length) {
      body = uiEmpty("Couldn't read your projects", state.projectsError, '<button type="button" class="btn btn-primary" data-action="projects-reload">Try again</button>');
    } else if (!all.length) {
      body = uiEmpty("No projects yet", "devd works with projects on your disk. Add a folder that has one, or start a new one from a stack.",
        '<div class="inline"><button type="button" class="btn btn-primary" data-action="nav" data-view="create">Create a project</button><button type="button" class="btn" data-action="add-project">Add a folder</button></div>');
    } else {
      var exposed = all.filter(function (p) { return (p.sites || []).length; }).length;
      var registered = all.filter(function (p) { return p.registered; }).length;
      var sorts = PROJECT_SORTS.map(function (s) {
        return '<button type="button" class="ui-sort' + (state.projectSort === s.id ? " active" : "") + '" data-action="project-sort" data-sort="' + attr(s.id) + '">' + esc(s.label) + "</button>";
      }).join("");
      var controls = '<div class="ui-controls"><input type="search" id="project-search" class="ui-search" placeholder="Search projects, stacks or site names" value="' + attr(state.projectQuery) + '" aria-label="Search projects"><span class="grow"></span><span class="sort-label">Sort</span>' + sorts + "</div>";
      // Agent work belongs to a project, so a project says when agents are on it.
      var agentWork = {};
      ((AI.home && AI.home.teamRuns) || []).forEach(function (t) {
        if (t.project && ["planning", "running", "awaiting_plan", "paused"].indexOf(t.state) >= 0) agentWork[t.project] = (agentWork[t.project] || 0) + 1;
      });
      ((AI.home && AI.home.threads) || []).forEach(function (t) {
        if (t.project && (t.state === "running" || t.state === "waiting_approval")) agentWork[t.project] = (agentWork[t.project] || 0) + 1;
      });
      var rows = visible.map(function (p) {
        var st = p.status;
        var facts = [];
        var bits = [];
        if (p.stack) facts.push('<span class="fact">' + esc(p.stack) + "</span>");
        var dotKind = "off";
        if (st) {
          var changes = projectChanges(st);
          facts.push('<span class="fact mono" title="Git branch">' + esc(st.branch || "detached") + "</span>");
          facts.push(changes ? '<span class="fact fact-warn">' + changes + " change" + (changes === 1 ? "" : "s") + "</span>" : '<span class="fact fact-ok">clean</span>');
          if (st.ahead) bits.push(st.ahead + " ahead");
          if (st.behind) bits.push(st.behind + " behind");
          dotKind = changes ? "warn" : "ok";
        } else if (p.is_repository) bits.push("repository");
        if (p.exists === false) { facts.push('<span class="fact fact-bad">folder is gone</span>'); dotKind = "bad"; }
        if (p.error) dotKind = "warn";
        if (!p.registered) bits.push(p.source === "site" ? "known from its site" : "not imported");
        var site = siteFor[p.path];
        if (site) facts.push('<span class="fact mono" title="The address that serves it">' + esc(site.site.host || site.site.name) + "</span>");
        if (agentWork[p.path]) facts.push('<span class="fact fact-ok">' + uiDot("live") + agentWork[p.path] + " agent" + (agentWork[p.path] === 1 ? "" : "s") + " working</span>");
        var isPinned = pinned.indexOf(p.path) !== -1;
        if (isPinned) facts.push('<span class="fact">pinned</span>');
        return uiRow({
          dot: dotKind, title: '<span class="row-name">' + esc(p.name) + "</span>" +
            (p.error ? ' <span class="fact fact-warn" title="' + attr(p.is_repository ? "Git cannot open this repository. " + p.error : gitErrorShort(p.error) || p.error) + '">Git unavailable</span>' : "") +
            (facts.length ? ' <span class="facts">' + facts.join("") + "</span>" : ""),
          sub: '<span class="mono" title="' + attr(p.path) + '">' + esc(p.path) + "</span>" + (bits.length ? " · " + esc(bits.join(" · ")) : ""),
          meta: site ? esc(servingLabel(site)) : "no address",
          open: 'data-action="open-project" data-path="' + attr(p.path) + '"',
          actions: '<button type="button" class="btn btn-mini btn-primary" data-action="open-project" data-path="' + attr(p.path) + '">Open</button>' +
            (site ? '<button type="button" class="btn btn-mini" data-action="open-site" data-name="' + attr(site.site.name) + '">Site</button>' : "") +
            '<button type="button" class="btn btn-mini btn-ghost" data-action="pin-project" data-path="' + attr(p.path) + '" title="' + (isPinned ? "Unpin" : "Pin to the top") + '">' + (isPinned ? "Unpin" : "Pin") + "</button>" +
            '<button type="button" class="btn btn-mini btn-ghost" data-action="project-forget" data-path="' + attr(p.path) + '" title="Remove it from devd; the folder is kept">Remove</button>',
          hover: true
        });
      });
      body = summaryBand([
        { label: "Projects", value: all.length + (all.length === 1 ? " project" : " projects"), detail: "in the folders devd knows" },
        { label: "With an address", value: String(exposed), detail: exposed === all.length ? "every project has a site" : (all.length - exposed) + " not exposed as a site" },
        { label: "Registered", value: String(registered), detail: registered === all.length ? "all imported into devd" : (all.length - registered) + " known only because a site serves them" }
      ]) + (all.length > 4 ? controls : "") + (visible.length ? uiList(rows) : uiEmpty('No project matches "' + state.projectQuery + '"', "Clear the search to see the rest."));
    }
    el("main").innerHTML = page("Projects", "",
      '<button type="button" class="btn" data-action="clone-form">Clone</button><button type="button" class="btn btn-primary" data-action="add-project">Add a folder</button>', body);
  }

  function loadProjects(quiet) {
    if (state.projectsPending) return state.projectsPending;
    state.projectsPending = read("project.list")
      .then(function (rows) {
        var list = Array.isArray(rows) ? rows : [];
        state.projectsLoaded = true;
        state.projectsError = "";
        state.projects = list.map(function (p) {
          return {
            path: p.path,
            name: p.name || dirName(p.path),
            stack: p.stack,
            package_manager: p.package_manager,
            source: p.source,
            registered: p.registered,
            is_repository: p.is_repository,
            exists: p.exists,
            sites: p.sites || [],
            updated_at: p.updated_at,
            status: null,
            error: null
          };
        });
        state.projectCount = state.projects.length;
        // The state of a repository is worth knowing, but a folder that is not
        // a repository is not an error - it simply has no Git state.
        return Promise.all(
          state.projects.map(function (p) {
            if (!p.is_repository) return Promise.resolve();
            return read("git.status", { repo: p.path })
              .then(function (st) { p.status = st; })
              .catch(function (e) { p.error = e.message; });
          })
        );
      })
      .catch(function (e) {
        state.projectsLoaded = true;
        state.projectsError = e.message;
        if (!quiet) {
          state.projects = [];
          toast("error", e.message);
        }
      })
      .then(function () { state.projectsPending = null; });
    return state.projectsPending;
  }

  // ------------------------------------------------------------------- project

  var TAB_LABELS = {
    changes: "Changes",
    history: "History",
    branches: "Branches",
    tags: "Tags",
    remotes: "Remotes",
    stashes: "Stashes",
    worktrees: "Worktrees",
    recover: "Recover",
    reviews: "Reviews",
    memory: "Agent memory"
  };

  function openProject(path) {
    state.view = "project";
    state.tab = "changes";
    state.tabs = {};
    state.diff = null;
    state.project = { path: path, name: dirName(path), status: null, open: null, error: null };
    writeRoute("project", false);
    render();
    return loadProject().then(render);
  }

  function loadProject() {
    var path = state.project.path;
    var jobs = [
      read("git.status", { repo: path }).then(
        function (st) { state.project.status = st; },
        function (e) { state.project.error = e.message; }
      ),
      read("git.repo.open", { repo: path }).then(
        function (info) { state.project.open = info; },
        function () {}
      )
    ];
    return Promise.all(jobs);
  }

  function loadTab(tab) {
    var path = state.project.path;
    var loaders = {
      changes: function () { return read("git.status", { repo: path }); },
      history: function () { return read("git.log", { repo: path, limit: 60 }); },
      branches: function () { return read("git.branch.list", { repo: path, all: true }); },
      tags: function () { return read("git.tag.list", { repo: path }); },
      remotes: function () { return read("git.remote.list", { repo: path }); },
      stashes: function () { return read("git.stash.list", { repo: path }); },
      worktrees: function () { return read("git.worktree.list", { repo: path }); },
      reviews: function () { return read("review.list", { limit: 30 }); },
      memory: function () { return read("memory.list", { limit: 30 }); },
      recover: function () {
        return Promise.all([
          read("git.reflog", { repo: path, limit: 50 }),
          read("git.conflicts.list", { repo: path })
        ]).then(function (both) {
          return { reflog: both[0], conflicts: both[1] };
        });
      }
    };
    state.tabs[tab] = { loading: true };
    return loaders[tab]()
      .then(function (data) {
        state.tabs[tab] = { loading: false, data: data };
      })
      .catch(function (e) {
        state.tabs[tab] = { loading: false, error: e.message };
      });
  }

  function fileRow(kind, change) {
    var path = typeof change === "string" ? change : change.path;
    // Git reports untracked and conflicted files as bare paths. The group the
    // file is in says which it is.
    var code = typeof change === "string" ? (kind === "conflicted" ? "U" : "?") : change.status;
    var actions = [];
    if (kind === "staged") {
      actions.push(mini("Diff", "diff-file", { path: path, staged: "1" }));
      actions.push(mini("Unstage", "unstage", { path: path }));
    } else if (kind === "unstaged") {
      actions.push(mini("Diff", "diff-file", { path: path }));
      actions.push(mini("Stage", "stage", { path: path }));
      actions.push(mini("Discard", "discard", { path: path }, { danger: true }));
    } else if (kind === "untracked") {
      actions.push(mini("Diff", "diff-file", { path: path }));
      actions.push(mini("Stage", "stage", { path: path }));
      actions.push(mini("Discard", "discard", { path: path, untracked: "1" }, { danger: true }));
    } else if (kind === "conflicted") {
      actions.push(mini("Show", "conflict-show", { path: path }));
      actions.push(mini("Ours", "resolve-ours", { path: path }));
      actions.push(mini("Theirs", "resolve-theirs", { path: path }));
    }
    var selected = state.diff && state.diff.path === path ? " selected" : "";
    return (
      '<div class="row' + selected + '">' +
      statusTag(code) +
      '<span class="row-main nowrap" title="' + attr(path) + '">' + esc(path) + "</span>" +
      '<span class="row-actions">' + actions.join("") + "</span>" +
      "</div>"
    );
  }

  function fileGroup(label, kind, items) {
    if (!items || !items.length) return "";
    return (
      '<div class="file-group">' +
      '<div class="file-group-head"><span>' + esc(label) + '</span><span class="right">' + items.length + "</span></div>" +
      items.map(function (c) { return fileRow(kind, c); }).join("") +
      "</div>"
    );
  }

  function renderChanges() {
    var st = state.project.status;
    if (!st) return '<div class="empty">Status is unavailable for this repository.</div>';

    var conflicts = (st.conflicted || []).length;
    var staged = (st.staged || []).length;
    var changed = (st.unstaged || []).length;
    var untracked = (st.untracked || []).length;
    var total = conflicts + staged + changed + untracked;

    var groups =
      fileGroup("Conflicted", "conflicted", st.conflicted) +
      fileGroup("Staged", "staged", st.staged) +
      fileGroup("Changed", "unstaged", st.unstaged) +
      fileGroup("Untracked", "untracked", st.untracked);

    if (!groups) groups = '<div class="empty"><div class="empty-title">Working tree clean</div>' +
      '<div class="empty-hint">Nothing to commit. New changes appear here as you work.</div></div>';

    // Say what is in the tree before listing it. The counts are the actual
    // groups below, not a second opinion about them.
    var summary = total
      ? '<div class="change-summary">' +
        (conflicts ? '<span class="pill pill-bad">' + conflicts + " conflicted</span>" : "") +
        (staged ? '<span class="pill pill-accent">' + staged + " staged</span>" : "") +
        (changed ? '<span class="pill pill-warn">' + changed + " changed</span>" : "") +
        (untracked ? '<span class="pill">' + untracked + " untracked</span>" : "") +
        "</div>"
      : "";

    var left =
      '<div class="panel"><div class="panel-head"><span>Working tree</span>' +
      '<span class="panel-note">' + esc(st.branch || "(detached)") +
      (total ? " · " + total + " file" + (total === 1 ? "" : "s") : "") + "</span>" +
      '<span class="inline right">' +
      mini("Stage all", "stage-all") +
      mini("Unstage all", "unstage-all") +
      mini("Stash changes", "stash-push") +
      "</span></div>" +
      '<div class="panel-body flush">' + summary + '<div class="rows">' + groups + "</div></div></div>";

    var diffPanel =
      '<div class="panel"><div class="panel-head"><span>Diff</span>' +
      (state.diff ? '<span class="panel-note mono nowrap" title="' + attr(state.diff.title) + '">' + esc(state.diff.title) + "</span>" : "") +
      "</div>" +
      (state.diff
        ? '<div class="panel-body flush">' + renderPatch(state.diff.patch, 3000) + "</div>"
        : '<div class="empty"><div class="empty-title">No file selected</div><div class="empty-hint">Choose a file from the working tree to read its diff.</div></div>') +
      "</div>";

    var canCommit = staged > 0;
    var commit =
      '<div class="panel"><div class="panel-head"><span>Commit</span>' +
      '<span class="panel-note">' + staged + " staged</span></div>" +
      '<div class="panel-body"><div class="commit-box">' +
      '<textarea class="textarea" id="commit-message" placeholder="' + (canCommit ? "Describe the change" : "Stage a file first") + '"' + (canCommit ? "" : " disabled") + "></textarea>" +
      '<div class="commit-row">' +
      '<label class="check"><input type="checkbox" id="commit-amend"' + (canCommit ? "" : " disabled") + "> Amend the last commit</label>" +
      '<button type="button" class="btn btn-primary" data-action="commit"' + (canCommit ? "" : " disabled") + ">Commit" + (staged ? " " + staged + " file" + (staged === 1 ? "" : "s") : "") + "</button>" +
      "</div>" +
      (canCommit ? "" : '<div class="row-sub">Only staged changes are committed. Stage a file to enable this.</div>') +
      "</div></div></div>";

    return '<div class="columns">' + left + "<div>" + diffPanel + commit + "</div></div>";
  }

  function renderHistory() {
    var data = state.tabs.history && state.tabs.history.data;
    if (!data) return '<div class="empty">No history loaded</div>';
    var commits = Array.isArray(data) ? data : [];
    if (!commits.length) return '<div class="empty">This repository has no commits yet</div>';
    return (
      '<div class="panel"><div class="panel-head"><span>History</span>' +
      '<span class="panel-note">newest first</span></div>' +
      '<div class="panel-body flush"><div class="rows">' +
      commits
        .map(function (c) {
          return (
            '<div class="row">' +
            '<span class="tag">' + esc(c.short.slice ? c.short.slice(0, 7) : c.short) + "</span>" +
            '<span class="row-main nowrap" title="' + attr(c.subject) + '">' + esc(c.subject) + "</span>" +
            '<span class="row-meta">' + esc(c.author) + "</span>" +
            '<span class="row-meta">' + esc(relativeTime(c.timestamp)) + "</span>" +
            '<span class="row-actions">' + mini("Show", "show-commit", { rev: c.hash }) +
            mini("Revert", "revert-commit", { rev: c.hash }, { danger: true }) +
            mini("Cherry-pick", "cherry-pick", { rev: c.hash }) + "</span>" +
            "</div>"
          );
        })
        .join("") +
      "</div></div></div>"
    );
  }

  function renderBranches() {
    var data = state.tabs.branches && state.tabs.branches.data;
    var branches = Array.isArray(data) ? data : [];

    var rows = branches.length
      ? branches
          .map(function (b) {
            // A remote-tracking entry is a name of the form `<remote>/<branch>`.
            // It is shown for reference but has no local actions, because
            // switching to it would detach HEAD.
            var remote = !b.current && /^[^/]+\/.+/.test(b.name) && !b.upstream;
            var actions = b.current
              ? []
              : remote
              ? [mini("Merge into current", "merge-current", { branch: b.name })]
              : [mini("Switch", "switch", { branch: b.name }),
                 mini("Merge into current", "merge-current", { branch: b.name }),
                 mini("Rename", "rename-branch", { from: b.name }),
                 mini("Delete", "delete-branch", { name: b.name }, { danger: true })];
            return (
              '<div class="row">' +
              (b.current ? '<span class="tag tag-add">*</span>' : '<span class="tag"> </span>') +
              '<span class="row-main nowrap"><span class="' + (b.current ? "branch-current" : "") + '">' + esc(b.name) + "</span>" +
              (remote ? ' <span class="faint">remote-tracking</span>' : "") + "</span>" +
              '<span class="row-meta">' + esc(b.upstream || "") + "</span>" +
              '<span class="row-meta nowrap" title="' + attr(b.subject) + '">' + esc((b.subject || "").slice(0, 48)) + "</span>" +
              '<span class="row-actions">' + actions.join("") + "</span>" +
              "</div>"
            );
          })
          .join("")
      : '<div class="empty">No branches</div>';

    return (
      '<div class="panel"><div class="panel-head"><span>Branches</span>' +
      '<span class="panel-note">' + branches.length + "</span></div>" +
      '<div class="panel-body flush"><div class="rows">' + rows + "</div></div>" +
      '<div class="panel-body inline">' +
      '<input class="input mono" id="new-branch" placeholder="new-branch-name" style="width:220px">' +
      '<button type="button" class="btn btn-primary" data-action="create-branch">Create and switch</button>' +
      "</div></div>"
    );
  }

  function renderTags() {
    var data = state.tabs.tags && state.tabs.tags.data;
    var tags = Array.isArray(data) ? data : [];
    var rows = tags.length
      ? tags
          .map(function (t) {
            return (
              '<div class="row">' +
              '<span class="row-main nowrap">' + esc(t.name) + "</span>" +
              '<span class="row-meta">' + esc((t.target || "").slice(0, 8)) + "</span>" +
              '<span class="row-meta nowrap">' + esc(t.message || "") + "</span>" +
              '<span class="row-actions">' +
              mini("Delete", "delete-tag", { name: t.name }, { danger: true }) +
              "</span></div>"
            );
          })
          .join("")
      : '<div class="empty">No tags</div>';

    return (
      '<div class="panel"><div class="panel-head"><span>Tags</span><span class="panel-note">' + tags.length + "</span></div>" +
      '<div class="panel-body flush"><div class="rows">' + rows + "</div></div>" +
      '<div class="panel-body inline">' +
      '<input class="input mono" id="new-tag" placeholder="v1.0.0" style="width:160px">' +
      '<input class="input" id="new-tag-message" placeholder="Annotation (optional)" style="width:280px">' +
      '<button type="button" class="btn btn-primary" data-action="create-tag">Create</button>' +
      '<button type="button" class="btn" data-action="push-tags">Push all tags</button>' +
      "</div>" +
      '<div class="panel-body flush" style="border-top:1px solid var(--line)">' +
      '<div class="dim" style="padding:8px 12px">Pushing tags publishes releases to the remote, so it asks for approval.</div>' +
      "</div></div>"
    );
  }

  function renderRemotes() {
    var data = state.tabs.remotes && state.tabs.remotes.data;
    var remotes = Array.isArray(data) ? data : [];
    var rows = remotes.length
      ? remotes
          .map(function (r) {
            return (
              '<div class="row">' +
              '<span class="row-main nowrap">' + esc(r.name) + "</span>" +
              '<span class="row-main nowrap faint" title="' + attr(r.fetch_url) + '">' + esc(r.fetch_url) + "</span>" +
              '<span class="row-actions">' +
              mini("Fetch", "fetch", { remote: r.name }) +
              mini("Pull", "pull", { remote: r.name }) +
              mini("Push", "push", { remote: r.name }) +
              mini("Remove", "remove-remote", { name: r.name }, { danger: true }) +
              "</span></div>"
            );
          })
          .join("")
      : '<div class="empty">No remotes configured</div>';

    return (
      '<div class="panel"><div class="panel-head"><span>Remotes</span>' +
      '<span class="panel-note">' + remotes.length + "</span></div>" +
      '<div class="panel-body flush"><div class="rows">' + rows + "</div></div>" +
      '<div class="panel-body inline">' +
      '<input class="input mono" id="remote-name" placeholder="origin" style="width:120px">' +
      '<input class="input mono" id="remote-url" placeholder="https://host/owner/repo.git" style="width:340px">' +
      '<button type="button" class="btn" data-action="add-remote">Add remote</button>' +
      "</div></div>" +
      '<div class="panel"><div class="panel-head"><span>Synchronise</span></div>' +
      '<div class="panel-body stack">' +
      '<div class="inline">' +
      '<select class="select" id="pull-strategy">' +
      '<option value="">default strategy</option>' +
      '<option value="merge">merge</option>' +
      '<option value="rebase">rebase</option>' +
      '<option value="fast-forward-only">fast-forward only</option>' +
      "</select>" +
      '<button type="button" class="btn btn-primary" data-action="pull-strategy">Pull from origin</button>' +
      '<button type="button" class="btn" data-action="push-origin">Push to origin</button>' +
      '<button type="button" class="btn" data-action="push-upstream">Push and set upstream</button>' +
      '<button type="button" class="btn" data-action="sync-preview">Preview sync</button>' +
      '<button type="button" class="btn btn-danger" data-action="force-push">Force push</button>' +
      "</div>" +
      '<div class="faint">Force push asks for approval every time and uses --force-with-lease.</div>' +
      "</div></div>"
    );
  }

  function renderStashes() {
    var data = state.tabs.stashes && state.tabs.stashes.data;
    var stashes = Array.isArray(data) ? data : [];
    var rows = stashes.length
      ? stashes
          .map(function (s) {
            return (
              '<div class="row">' +
              '<span class="tag">' + esc(s.reference) + "</span>" +
              '<span class="row-main nowrap">' + esc(s.subject) + "</span>" +
              '<span class="row-actions">' +
              mini("Show", "stash-show", { ref: s.reference }) +
              mini("Apply", "stash-apply", { ref: s.reference }) +
              mini("Pop", "stash-pop", { ref: s.reference }) +
              mini("Drop", "stash-drop", { ref: s.reference }, { danger: true }) +
              "</span></div>"
            );
          })
          .join("")
      : '<div class="empty">No stashes</div>';

    return (
      '<div class="panel"><div class="panel-head"><span>Stashes</span>' +
      '<span class="panel-note">' + stashes.length + "</span></div>" +
      '<div class="panel-body flush"><div class="rows">' + rows + "</div></div>" +
      '<div class="panel-body inline">' +
      '<input class="input" id="stash-message" placeholder="Why are you shelving this?" style="width:340px">' +
      '<label class="check"><input type="checkbox" id="stash-untracked"> Include untracked files</label>' +
      '<button type="button" class="btn btn-primary" data-action="stash-push">Stash changes</button>' +
      "</div></div>"
    );
  }

  function renderWorktrees() {
    var data = state.tabs.worktrees && state.tabs.worktrees.data;
    var worktrees = Array.isArray(data) ? data : [];
    var rows = worktrees
      .map(function (w) {
        return (
          '<div class="row">' +
          '<span class="tag">' + (w.main ? "main" : "wt") + "</span>" +
          '<span class="row-main nowrap" title="' + attr(w.path) + '">' + esc(w.path) + "</span>" +
          '<span class="row-meta">' + esc(w.branch || "(detached)") + "</span>" +
          '<span class="row-meta">' + esc((w.head || "").slice(0, 8)) + "</span>" +
          '<span class="row-actions">' +
          (w.main ? "" : mini("Remove", "worktree-remove", { path: w.path }, { danger: true })) +
          "</span></div>"
        );
      })
      .join("");

    return (
      '<div class="panel"><div class="panel-head"><span>Worktrees</span>' +
      '<span class="panel-note">' + worktrees.length + "</span></div>" +
      '<div class="panel-body flush"><div class="rows">' + rows + "</div></div>" +
      '<div class="panel-body">' +
      '<div class="inline">' +
      '<input class="input mono" id="worktree-path" placeholder="C:\\\\code\\\\my-project-feature" style="width:320px">' +
      '<input class="input mono" id="worktree-branch" placeholder="feature-branch" style="width:180px">' +
      '<label class="check"><input type="checkbox" id="worktree-create" checked> create the branch</label>' +
      '<button type="button" class="btn btn-primary" data-action="worktree-add">Add worktree</button>' +
      '<button type="button" class="btn" data-action="worktree-prune">Prune stale</button>' +
      "</div>" +
      '<div class="faint" style="margin-top:6px">A worktree is a second checkout of the same repository. ' +
      "It is the safe way to run a long task without disturbing the changes in this one.</div>" +
      "</div></div>"
    );
  }

  function renderRecover() {
    var tab = state.tabs.recover || {};
    var data = tab.data || {};
    var st = state.project.status || {};
    var parts = [];

    if (st.in_progress) {
      var canContinue = st.in_progress !== "bisect";
      parts.push(
        '<div class="notice notice-warn"><div class="grow"><strong>A ' + esc(st.in_progress) +
        " is in progress.</strong> Resolve it before doing anything else.</div>" +
        (canContinue ? mini("Continue", "op-continue", { op: st.in_progress }) : "") +
        mini("Abort", "op-abort", { op: st.in_progress }, { danger: true }) +
        "</div>"
      );
    }

    var conflicts = (data.conflicts && data.conflicts.conflicted) || [];
    if (conflicts.length) {
      parts.push(
        '<div class="panel"><div class="panel-head"><span>Conflicts</span>' +
        '<span class="panel-note">' + conflicts.length + "</span></div>" +
        '<div class="panel-body flush"><div class="rows">' +
        conflicts
          .map(function (p) {
            return (
              '<div class="row">' + statusTag("U") +
              '<span class="row-main nowrap">' + esc(p) + "</span>" +
              '<span class="row-actions">' +
              mini("Show", "conflict-show", { path: p }) +
              mini("Keep ours", "resolve-ours", { path: p }) +
              mini("Keep theirs", "resolve-theirs", { path: p }) +
              mini("Stage resolved", "stage", { path: p }) +
              "</span></div>"
            );
          })
          .join("") +
        "</div></div></div>"
      );
    }

    var entries = (data.reflog && data.reflog.entries) || [];
    parts.push(
      '<div class="panel"><div class="panel-head"><span>Reflog</span>' +
      '<span class="panel-note">where HEAD has been</span></div>' +
      '<div class="panel-body flush"><div class="rows">' +
      (entries.length
        ? entries
            .map(function (e) {
              var hash = e.hash || "";
              var subject = e.subject || "";
              return (
                '<div class="row">' +
                '<span class="tag">' + esc(String(hash).slice(0, 7)) + "</span>" +
                '<span class="row-main">' +
                (e.action ? '<span class="pill">' + esc(e.action) + "</span> " : "") +
                esc(subject) +
                "</span>" +
                '<span class="row-meta mono nowrap" title="' + attr(e.timestamp || "") + '">' +
                esc(String(e.timestamp || "").slice(0, 16)) + "</span>" +
                '<span class="row-actions">' +
                mini("Reset to this", "reset-to", { rev: hash }) +
                "</span></div>"
              );
            })
            .join("")
        : '<div class="empty">The reflog is empty</div>') +
      "</div></div></div>"
    );

    parts.push(
      '<div class="panel"><div class="panel-head"><span>History repair</span></div>' +
      '<div class="panel-body stack">' +
      '<div class="inline">' +
      '<input class="input mono" id="recovery-rev" placeholder="a revision: HEAD~2, a hash, origin/main" style="width:300px">' +
      '<select class="select" id="reset-mode">' +
      '<option value="soft">soft - keep index and files</option>' +
      '<option value="mixed">mixed - keep files</option>' +
      '<option value="hard">hard - discard everything</option>' +
      "</select>" +
      '<button type="button" class="btn" data-action="reset-mode">Reset</button>' +
      '<button type="button" class="btn" data-action="rebase-onto">Rebase onto</button>' +
      "</div>" +
      '<div class="inline">' +
      '<input class="input mono" id="squash-rev" placeholder="count of recent commits" style="width:170px" value="2">' +
      '<input class="input" id="squash-message" placeholder="message for the squashed commit" style="width:320px">' +
      '<button type="button" class="btn" data-action="squash">Squash</button>' +
      "</div>" +
      '<div class="inline">' +
      '<input class="input mono" id="bisect-good" placeholder="good revision" style="width:200px">' +
      '<input class="input mono" id="bisect-bad" placeholder="bad revision" style="width:200px">' +
      '<button type="button" class="btn" data-action="bisect-start">Start bisect</button>' +
      '<button type="button" class="btn" data-action="bisect-good">Mark good</button>' +
      '<button type="button" class="btn" data-action="bisect-bad">Mark bad</button>' +
      '<button type="button" class="btn" data-action="bisect-skip">Skip</button>' +
      '<button type="button" class="btn" data-action="bisect-reset">End bisect</button>' +
      "</div>" +
      '<div class="faint">A hard reset discards uncommitted work and asks for approval. ' +
      "The reflog is the way back if it was a mistake.</div>" +
      "</div></div>"
    );

    return parts.join("");
  }

  /* What this repository is connected to, from what devd really knows: its code
     (stack, branch, state), the site that serves it, what runs it, and the agent
     runs that name it. Each reading leads to where it can be acted on, and a
     reading devd has no record of says so instead of being left out. */
  function projectContext(path) {
    var known = (state.projects || []).filter(function (x) { return x.path === path && x.stack; })[0] ||
      ((AI.home && AI.home.projects) || []).filter(function (x) { return x.path === path; })[0] || {};
    var st = state.project && state.project.status;
    var site = (state.server.sites || []).filter(function (r) { return r.project === path; })[0];
    var runs = ((AI.home && AI.home.teamRuns) || []).filter(function (t) { return t.project === path; });
    var active = runs.filter(function (t) { return ["planning", "running", "awaiting_plan", "paused"].indexOf(t.state) >= 0; });
    var cells = [];

    var codeBits = [];
    if (st) {
      var changed = st.staged.length + st.unstaged.length + st.untracked.length;
      codeBits.push(esc(st.branch || "detached"));
      codeBits.push(changed ? '<span class="ov-warn">' + changed + (changed === 1 ? " change" : " changes") + "</span>" : "clean");
      if (st.ahead || st.behind) codeBits.push(st.ahead + " ahead, " + st.behind + " behind");
      if (st.conflicted.length) codeBits.push('<span class="ov-bad">' + st.conflicted.length + " conflicted</span>");
      if (st.in_progress) codeBits.push('<span class="ov-warn">' + esc(st.in_progress) + "</span>");
    }
    cells.push('<div class="ov-cell"><div class="ov-k">Code</div><div class="ov-v">' + esc(known.stack || "Stack not detected") + '</div><div class="ov-d">' + (codeBits.join(" · ") || "reading git status") + "</div></div>");

    if (!state.server.loaded) {
      cells.push('<div class="ov-cell"><div class="ov-k">Serves at</div><div class="ov-v ov-wait">Reading sites…</div></div>');
      cells.push('<div class="ov-cell"><div class="ov-k">Runs on</div><div class="ov-v ov-wait">Reading sites…</div></div>');
    } else if (site) {
      var attn = siteNeedsAttention(site) ? '<span class="ov-warn">' + esc(siteAttentionReason(site)) + "</span>" : esc(servingLabel(site) || "not serving");
      cells.push('<button type="button" class="ov-cell ov-link" data-action="open-site" data-name="' + attr(site.site.name) + '"><div class="ov-k">Serves at</div><div class="ov-v mono">' + esc(site.site.host || site.site.name) + '</div><div class="ov-d">' + attn + (site.certificate ? " · HTTPS certificate" : site.site.tls_mode !== "off" ? ' · <span class="ov-warn">no certificate</span>' : " · HTTP only") + "</div></button>");
      var run;
      if (site.serving === "php") run = ['PHP ' + esc(site.php_effective || ""), site.php_source === "pinned" ? "pinned for this site" : "follows the machine default"];
      else if (site.serving === "proxy") run = ["Proxy", esc(site.site.upstream || "") + (site.upstream_listening ? " · listening" : ' · <span class="ov-warn">not running</span>')];
      else if (site.serving === "static") run = ["Static files", "no runtime needed"];
      else run = ["Nothing to run", esc(site.detected || "no web entry point")];
      cells.push('<div class="ov-cell"><div class="ov-k">Runs on</div><div class="ov-v">' + run[0] + '</div><div class="ov-d">' + run[1] + "</div></div>");
    } else {
      cells.push('<div class="ov-cell"><div class="ov-k">Serves at</div><div class="ov-v">No address yet</div><div class="ov-d">no site is recorded for this folder</div></div>');
      cells.push('<div class="ov-cell"><div class="ov-k">Runs on</div><div class="ov-v">Not served</div><div class="ov-d">give it a site to see its runtime</div></div>');
    }

    cells.push('<button type="button" class="ov-cell ov-link" data-action="nav" data-view="team" data-exact="1"><div class="ov-k">Agent work</div><div class="ov-v">' +
      (runs.length ? runs.length + (runs.length === 1 ? " run" : " runs") : "No runs") + '</div><div class="ov-d">' +
      (active.length ? active.length + " in progress or waiting for you" : runs.length ? "none in progress" : "none have named this project") + "</div></button>");
    return '<div class="ov" aria-label="What this project is connected to">' + cells.join("") + "</div>";
  }

  function renderProject() {
    if (state.view !== "project") return;
    var st = state.project.status;
    var tab = state.tab;
    var tabs = Object.keys(TAB_LABELS)
      .map(function (name) {
        return (
          '<button type="button" class="tab' + (tab === name ? " active" : "") +
          '" data-action="tab" data-tab="' + name + '">' + esc(TAB_LABELS[name]) + "</button>"
        );
      })
      .join("");

    var current = state.tabs[tab] || {};
    if (tab === "reviews" || tab === "memory") {
      if (STUDIO.project !== state.project.path) {
        STUDIO.selected = null;
        STUDIO.memoryOpen = null;
        STUDIO.compat = null;
      }
      STUDIO.project = state.project.path;
      if (tab === "reviews") STUDIO.reviews = current.data || [];
      else STUDIO.memory = current.data || [];
    }
    var body;
    if (current.loading) body = '<div class="loading">Loading</div>';
    else if (current.error) body = '<div class="notice notice-bad">' + esc(current.error) + "</div>";
    else if (tab === "changes") body = renderChanges();
    else if (tab === "history") body = renderHistory();
    else if (tab === "branches") body = renderBranches();
    else if (tab === "tags") body = renderTags();
    else if (tab === "remotes") body = renderRemotes();
    else if (tab === "stashes") body = renderStashes();
    else if (tab === "worktrees") body = renderWorktrees();
    else if (tab === "recover") body = renderRecover();
    else if (tab === "reviews") body = renderReviewTab(true);
    else if (tab === "memory") body = renderMemoryTab(true);
    else body = "";

    el("main").innerHTML =
      '<div class="page">' +
      '<div class="page-head">' +
      "<div>" +
      pageTitle(state.project.name, "project") +
      '<div class="page-sub mono nowrap" title="' + attr(state.project.path) + '">' +
      esc(state.project.path) + "</div>" +
      (state.project.error ? '<div class="notice notice-bad" style="margin-top:8px">' +
        esc(state.project.error) + "</div>" : "") +
      "</div>" +
      '<div class="page-actions">' +
      '<button type="button" class="btn" data-action="open-terminal" data-path="' + attr(state.project.path) + '" title="Open your terminal in this project, with its pinned runtimes and devd first on PATH">Terminal</button>' +
      '<button type="button" class="btn" data-action="project-refresh">Refresh</button>' +
      '<button type="button" class="btn btn-ghost" data-action="untrack">Remove from devd</button>' +
      '<button type="button" class="btn btn-ghost btn-danger" data-action="delete-folder">Delete from disk</button>' +
      "</div></div>" +
      projectContext(state.project.path) +
      '<div class="tabs">' + tabs + "</div>" +
      body +
      "</div>";
  }

  // -------------------------------------------------------------- capabilities

  function openCapability(id, args) {
    state.view = "capabilities";
    state.selectedCap = id;
    state.capArgs = args || {};
    render();
    return request("/api/capabilities/" + encodeURIComponent(id))
      .then(function (res) {
        if (res.ok) {
          state.selectedCapDetail = res.body;
          if (!Object.keys(state.capArgs).length) state.capArgs = skeleton(res.body.args_schema);
          render();
        }
      });
  }

  // ----------------------------------------------------------- argument forms
  //
  // A capability's schema says what it takes. The form below is that schema made
  // fillable: one labelled control per property, the type deciding the control, the
  // description under it. What the form cannot express (a nested object, a list of
  // objects) falls back to a JSON box for that one field, so nothing is unreachable.
  // The JSON view of the whole argument object stays one click away.

  function schemaType(prop) {
    if (!prop) return "string";
    var t = prop.type;
    if (Array.isArray(t)) {
      t = t.filter(function (x) { return x !== "null"; })[0] || "string";
    }
    if (!t && prop.anyOf) {
      var first = prop.anyOf.filter(function (x) { return x && x.type !== "null"; })[0];
      return schemaType(first);
    }
    if (!t && prop.enum) return "string";
    return t || "json";
  }

  function schemaEnum(prop) {
    if (!prop) return null;
    if (prop.enum && prop.enum.length) return prop.enum;
    if (prop.anyOf) {
      for (var i = 0; i < prop.anyOf.length; i++) {
        var e = schemaEnum(prop.anyOf[i]);
        if (e) return e;
      }
    }
    return null;
  }

  function hasForm(schema) {
    return !!(schema && schema.properties && Object.keys(schema.properties).length);
  }

  function argMode(schema) {
    if (!hasForm(schema)) return "json";
    return state.capMode === "json" ? "json" : "form";
  }

  function argModeTabs(schema) {
    if (!hasForm(schema)) return "";
    var mode = argMode(schema);
    return '<span class="inline right" role="tablist" aria-label="How to edit the arguments">' +
      '<button type="button" role="tab" class="ui-sort' + (mode === "form" ? " active" : "") + '" aria-selected="' + (mode === "form") + '" data-action="cap-mode" data-mode="form">Form</button>' +
      '<button type="button" role="tab" class="ui-sort' + (mode === "json" ? " active" : "") + '" aria-selected="' + (mode === "json") + '" data-action="cap-mode" data-mode="json">JSON</button></span>';
  }

  var LONG_TEXT = /^(content|script|body|diff|patch|message|prompt|instructions|text|description|sql|query|command)$/;

  function argField(key, prop, value, required) {
    var id = "cap-f-" + key;
    var t = schemaType(prop);
    var choices = schemaEnum(prop);
    var help = prop && prop.description ? '<div class="faint arg-help" id="' + attr(id) + '-help">' + esc(prop.description) + "</div>" : "";
    var described = help ? ' aria-describedby="' + attr(id) + '-help"' : "";
    var req = required ? ' required aria-required="true"' : "";
    var control;
    if (t === "boolean") {
      return '<div class="arg-field arg-bool"><label class="arg-check"><input type="checkbox" id="' + attr(id) + '" data-arg="' + attr(key) + '" data-type="boolean"' +
        (value === true ? " checked" : "") + described + "> <span>" + esc(key) + "</span></label>" + help + "</div>";
    }
    if (choices) {
      control = '<select class="input" id="' + attr(id) + '" data-arg="' + attr(key) + '" data-type="string"' + req + described + ">" +
        (required ? "" : '<option value=""></option>') +
        choices.map(function (c) {
          return '<option value="' + attr(String(c)) + '"' + (value === c ? " selected" : "") + ">" + esc(String(c)) + "</option>";
        }).join("") + "</select>";
    } else if (t === "integer" || t === "number") {
      control = '<input class="input mono" type="number" ' + (t === "integer" ? 'step="1"' : 'step="any"') + ' id="' + attr(id) + '" data-arg="' + attr(key) + '" data-type="' + t + '"' +
        (value === undefined || value === null ? "" : ' value="' + attr(String(value)) + '"') + req + described + ">";
    } else if (t === "array" && schemaType(prop.items) === "string") {
      control = '<textarea class="textarea mono arg-list" rows="3" id="' + attr(id) + '" data-arg="' + attr(key) + '" data-type="strings" spellcheck="false" placeholder="One per line"' + req + described + ">" +
        esc(Array.isArray(value) ? value.join("\n") : "") + "</textarea>";
    } else if (t === "string") {
      control = LONG_TEXT.test(key)
        ? '<textarea class="textarea mono" rows="4" id="' + attr(id) + '" data-arg="' + attr(key) + '" data-type="string" spellcheck="false"' + req + described + ">" + esc(value === undefined || value === null ? "" : String(value)) + "</textarea>"
        : '<input class="input mono" type="text" id="' + attr(id) + '" data-arg="' + attr(key) + '" data-type="string" spellcheck="false"' +
          (value === undefined || value === null ? "" : ' value="' + attr(String(value)) + '"') + req + described + ">";
    } else {
      control = '<textarea class="textarea mono" rows="3" id="' + attr(id) + '" data-arg="' + attr(key) + '" data-type="json" spellcheck="false" placeholder="JSON"' + req + described + ">" +
        esc(value === undefined || value === null ? "" : JSON.stringify(value, null, 2)) + "</textarea>";
    }
    return '<div class="arg-field"><label for="' + attr(id) + '">' + esc(key) +
      (required ? ' <span class="faint">required</span>' : "") + "</label>" + control + help + "</div>";
  }

  function argForm(schema, args) {
    var required = schema.required || [];
    return '<div class="arg-form" id="cap-form">' +
      Object.keys(schema.properties).map(function (key) {
        return argField(key, schema.properties[key], (args || {})[key], required.indexOf(key) !== -1);
      }).join("") + "</div>";
  }

  /* The argument object the form describes. With `quiet` a mistake yields what could be
     read instead of a message, which is how typing is mirrored into state. */
  function readArgForm(quiet) {
    var form = el("cap-form");
    if (!form) return {};
    var out = {};
    var bad = null;
    Array.prototype.forEach.call(form.querySelectorAll("[data-arg]"), function (node) {
      var key = node.dataset.arg, type = node.dataset.type, raw;
      node.removeAttribute("aria-invalid");
      if (type === "boolean") { out[key] = node.checked; return; }
      raw = node.value;
      if (raw.trim() === "") {
        if (node.required && !quiet) {
          node.setAttribute("aria-invalid", "true");
          if (!bad) bad = [node, key + " is required"];
        }
        return;
      }
      if (type === "integer" || type === "number") {
        var n = Number(raw);
        if (!isFinite(n) || (type === "integer" && Math.floor(n) !== n)) {
          if (!quiet && !bad) { bad = [node, key + " must be " + (type === "integer" ? "a whole number" : "a number")]; node.setAttribute("aria-invalid", "true"); }
          return;
        }
        out[key] = n;
      } else if (type === "strings") {
        out[key] = raw.split("\n").map(function (l) { return l.trim(); }).filter(Boolean);
      } else if (type === "json") {
        try { out[key] = JSON.parse(raw); }
        catch (e) { if (!quiet && !bad) { bad = [node, key + " is not valid JSON"]; node.setAttribute("aria-invalid", "true"); } }
      } else {
        out[key] = raw;
      }
    });
    if (bad) {
      toast("error", bad[1]);
      bad[0].focus();
      return null;
    }
    return out;
  }

  /* Build a starting argument object from a JSON Schema.
     Only fills in fields with an obvious trivial value; everything else is left
     out so the user sees exactly what they must supply. */
  function skeleton(schema) {
    var out = {};
    if (!schema || !schema.properties) return out;
    Object.keys(schema.properties).forEach(function (key) {
      var prop = schema.properties[key] || {};
      var t = prop.type;
      if (t === "boolean") out[key] = false;
      else if (t === "array") out[key] = [];
      else if (t === "string" && prop.enum && prop.enum.length) out[key] = prop.enum[0];
      else if (t === "integer" || t === "number") out[key] = 0;
    });
    return out;
  }

  function renderCapabilities() {
    if (state.view !== "capabilities") return;
    var detail = state.selectedCapDetail;
    var page = state.caps.page || 0;
    var start = page * PAGE_SIZE;
    var visible = state.caps.items.slice(start, start + PAGE_SIZE);
    var rows = visible
      .map(function (c) {
        return (
          '<div class="row' + (state.selectedCap === c.id ? " selected" : "") + '">' +
          '<span class="row-main nowrap"><button type="button" class="btn btn-ghost btn-mini" data-action="pick-cap" data-id="' +
          attr(c.id) + '">' + esc(c.id) + "</button></span>" +
          '<span class="row-meta">' + esc(c.tier) + "</span>" +
          "</div>"
        );
      })
      .join("");

    var inspector;
    if (!state.selectedCap || !detail) {
      inspector = '<div class="empty"><div class="empty-title">Pick a capability</div>' +
        "<div>Every entry in the registry is reachable here, with its real schema.</div></div>";
    } else {
      var flags = [];
      if (detail.requires_approval) flags.push('<span class="pill pill-bad">approval required</span>');
      if (detail.human_only) flags.push('<span class="pill pill-warn">human only</span>');
      if (!detail.agent_safe) flags.push('<span class="pill pill-warn">agents refused</span>');
      if (detail.supports_dry_run) flags.push('<span class="pill">plannable</span>');
      if (detail.idempotent) flags.push('<span class="pill">idempotent</span>');

      inspector =
        '<div class="panel"><div class="panel-head"><span class="mono">' + esc(detail.id) + "</span>" +
        '<span class="inline right"><span class="' + tierClass(detail.tier) + '">' + esc(detail.tier) +
        '</span><span class="pill">' + esc(detail.effect) + "</span></span></div>" +
        '<div class="panel-body stack">' +
        "<div>" + esc(detail.summary) + "</div>" +
        (detail.description ? '<div class="dim">' + esc(detail.description) + "</div>" : "") +
        '<div class="inline">' + flags.join("") + "</div>" +
        "</div></div>" +
        '<div class="panel"><div class="panel-head"><span>Arguments</span>' + argModeTabs(detail.args_schema) + "</div>" +
        '<div class="panel-body stack">' +
        (argMode(detail.args_schema) === "form"
          ? argForm(detail.args_schema, state.capArgs)
          : '<textarea class="textarea mono" id="cap-args" spellcheck="false" aria-label="Arguments as JSON">' +
            esc(JSON.stringify(state.capArgs, null, 2)) + "</textarea>") +
        '<div class="inline">' +
        '<button type="button" class="btn" data-action="cap-dry-run">Dry run</button>' +
        '<button type="button" class="btn ' + (detail.requires_approval ? "btn-danger" : "btn-primary") +
        '" data-action="cap-run">Run</button>' +
        '<span class="faint">The form is built from the capability’s schema. The registry checks the arguments again before anything runs.</span>' +
        "</div></div></div>" +
        '<div class="panel"><div class="panel-head"><span>Schema</span></div>' +
        '<div class="panel-body"><pre class="patch-block">' +
        esc(JSON.stringify(detail.args_schema, null, 2)) + "</pre></div></div>";
    }

    el("main").innerHTML =
      '<div class="page">' +
      '<div class="page-head"><div>' + pageTitle("API and tools", "capabilities") +
      '<div class="page-sub">' + state.caps.total + " of " +
      ((state.session && state.session.registry.capabilities) || 0) +
      " - the same registry the CLI and the agent use.</div></div></div>" +
      '<div class="columns">' +
      '<div class="panel"><div class="panel-head"><span>Registry</span><span class="panel-note">' +
      state.caps.items.length + " shown</span>" +
      '<input class="input mono cap-search" id="cap-search" placeholder="Filter capabilities" value="' +
      attr(state.caps.q) + '" aria-label="Filter capabilities"></div>' +
      '<div class="panel-body flush"><div class="rows">' +
      (rows || '<div class="empty-inline">No capability matches that filter.</div>') +
      "</div></div>" +
      pager(state.caps.items.length, page, "cap-page") +
      "</div>" +
      "<div>" + inspector + "</div>" +
      "</div></div>";
  }

  function loadCapabilities() {
    var url = "/api/capabilities?limit=500&q=" + encodeURIComponent(state.caps.q) +
      (state.caps.group ? "&group=" + encodeURIComponent(state.caps.group) : "");
    return request(url).then(function (res) {
      if (!res.ok) {
        toast("error", errorOf(res.body, "capability list failed"));
        return;
      }
      state.caps.total = res.body.total;
      state.caps.items = res.body.items || [];
      var pages = Math.max(1, Math.ceil(state.caps.items.length / PAGE_SIZE));
      if ((state.caps.page || 0) >= pages) state.caps.page = 0;
    });
  }

  // -------------------------------------------------------------------- activity

  var AUDIT_FILTERS = [["changes", "Changes"], ["mine", "What I did"], ["agents", "What agents did"], ["refused", "Turned down"], ["all", "Everything, including reads"]];

  function auditVisible() {
    var f = state.auditFilter || "changes";
    return state.audit.filter(function (a) {
      if (f === "all") return true;
      if (f === "mine") return a.actor.indexOf("agent") !== 0 && a.tier !== "read";
      if (f === "agents") return a.actor.indexOf("agent") === 0;
      if (f === "refused") return !a.ok && !a.dry_run;
      return !a.dry_run && a.tier !== "read";
    });
  }

  /* Where an operation's effects can be seen. The mapping is by capability family;
     an operation with no family here simply has no link, rather than a wrong one. */
  function activityDest(cap) {
    var c = String(cap || "");
    var map = [
      [/^(service|stack|web)\./, "services", "Services"], [/^(site|tls|hosts|dns|share)\./, "sites", "Sites"],
      [/^(team|run|schedule)\./, "team", "Agent runs"], [/^(project|git|fs|review|memory)\./, "projects", "Projects"],
      [/^(provider|models)\./, "providers", "Models"], [/^(runtime|php|toolchain)\./, "runtimes", "Runtimes"],
      [/^(approval|inbox)\./, "inbox", "Needs you"], [/^(mcp|capability|tools)\./, "capabilities", "API and tools"]
    ];
    for (var i = 0; i < map.length; i++) if (map[i][0].test(c)) return { view: map[i][1], label: map[i][2] };
    return null;
  }

  /* An operation in plain words: "service.start" reads "Started a service". The
     capability stays beside it, exact, for anyone who needs to search for it. A
     name devd has no sentence for is shown as it is, never guessed at. */
  var ACT_VERB = { start: "Started", stop: "Stopped", restart: "Restarted", install: "Installed", remove: "Removed", delete: "Deleted", import: "Imported", create: "Created", save: "Saved", write: "Wrote", edit: "Edited", read: "Read", run: "Ran", verify: "Checked", approve: "Approved", deny: "Declined", sync: "Updated", issue: "Issued", trust: "Trusted", untrust: "Stopped trusting", retry: "Retried", resume: "Resumed", pause: "Paused", cancel: "Cancelled", say: "Told", plan_submit: "Submitted a plan for", report: "Reported on", set_startup: "Changed startup for", test: "Tested", discover: "Looked for", use: "Switched to", list: "Listed", status: "Checked the status of", get: "Read", show: "Looked at" };
  var ACT_NOUN = { service: "a service", stack: "the whole stack", site: "a site", tls: "a certificate", hosts: "the hostnames", dns: "local DNS", project: "a project", fs: "a file", git: "a repository", team: "an agent run", run: "an agent run", provider: "a model provider", models: "the model setup", runtime: "a runtime", php: "PHP", approval: "an approval request", inbox: "a message", mcp: "an MCP tool", web: "the web server", share: "a shared link", schedule: "a schedule", environment: "the environment", health: "devd's health", template: "the run templates" };
  function activityWords(cap) {
    var m = String(cap || "").split(".");
    var verb = ACT_VERB[m.slice(1).join("_")] || ACT_VERB[m[1]];
    var noun = ACT_NOUN[m[0]];
    if (verb === "Listed" && noun && /^an? /.test(noun)) noun = noun.replace(/^an? /, "") + "s";
    return verb && noun ? verb + " " + noun : "";
  }
  function activityWho(actor) {
    var a = String(actor || "");
    if (a.indexOf("agent:") === 0) return "Agent " + a.replace("agent:", "").replace("devd-", "");
    if (a.indexOf("human:") === 0) { var via = a.replace("human:", ""); return via === "cli" ? "You, from the terminal" : via === "http" || via === "ui" ? "You, in the app" : "You (" + via + ")"; }
    return a;
  }

  function renderActivity() {
    if (state.view !== "activity") return;
    var pageNo = state.auditPage || 0;
    var visible = auditVisible();
    var start = pageNo * PAGE_SIZE;
    // The record reads as a day-by-day timeline: who did what, and whether it held.
    var lastDay = "";
    var rows = [];
    visible.slice(start, start + PAGE_SIZE).forEach(function (a) {
      var day = auditDay(a.at);
      if (day !== lastDay) { rows.push('<div class="ui-day" role="heading" aria-level="3">' + esc(day) + "</div>"); lastDay = day; }
      var words = activityWords(a.capability);
      var outcome = a.dry_run ? '<span class="fact">only previewed</span>' : a.ok ? '<span class="fact fact-ok">went through</span>' : '<span class="fact fact-bad">was refused</span>';
      rows.push(uiRow({
        dot: a.dry_run ? "off" : a.ok ? "ok" : "bad",
        title: '<span class="row-name">' + esc(words || a.capability) + "</span>" +
          ' <span class="facts">' + outcome + '<span class="fact">' + esc(activityWho(a.actor)) + "</span>" +
          (words ? '<span class="fact mono">' + esc(a.capability) + "</span>" : "") + "</span>",
        sub: a.message && a.message !== "ok" ? esc(uiSummary(a.message, 160)) : "",
        meta: '<time datetime="' + attr(a.at) + '" title="' + attr(stamp(a.at)) + '">' + esc(auditTime(a.at)) + "</time>",
        open: 'data-action="open-audit" data-at="' + attr(a.at) + '" data-cap="' + attr(a.capability) + '"',
        actions: (function () { var d = activityDest(a.capability); return d ? '<button type="button" class="btn btn-mini btn-ghost" data-action="nav" data-view="' + d.view + '"' + (d.view === "team" ? ' data-exact="1"' : "") + ' aria-label="Open ' + attr(d.label) + ' for ' + attr(a.capability) + '">' + esc(d.label) + "</button>" : ""; })(),
        hover: true
      }));
    });
    var tabs = '<div class="ui-controls">' + AUDIT_FILTERS.map(function (f) {
      return '<button type="button" class="ui-sort' + ((state.auditFilter || "changes") === f[0] ? " active" : "") + '" data-action="audit-filter" data-filter="' + f[0] + '">' + f[1] + "</button>";
    }).join("") + '<span class="grow"></span><button type="button" class="btn btn-mini btn-ghost" data-action="refresh">Refresh</button></div>';
    var body = tabs + (rows.length ? uiList(rows) + pager(visible.length, pageNo, "audit-page")
      : state.auditError ? uiEmpty("The activity log could not be read", state.auditError)
      : uiEmpty("Nothing to look back on yet", "Every change devd makes, whether you, an agent or a script asked for it, is written down here the moment it happens."));
    el("main").innerHTML = page("Activity", "A written record of what changed on this machine, who asked, and whether it went through.", "", body);
  }

  function auditDay(at) {
    var d = new Date(at);
    if (isNaN(d)) return "Unknown date";
    var today = new Date(); today.setHours(0, 0, 0, 0);
    var that = new Date(d); that.setHours(0, 0, 0, 0);
    var diff = Math.round((today - that) / 86400000);
    if (diff === 0) return "Today";
    if (diff === 1) return "Yesterday";
    return d.toLocaleDateString([], { weekday: "long", month: "long", day: "numeric", year: d.getFullYear() === today.getFullYear() ? undefined : "numeric" });
  }

  function auditTime(at) {
    var d = new Date(at);
    return isNaN(d) ? String(at) : d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }

  function openAuditSheet(at, cap) {
    var a = state.audit.filter(function (x) { return x.at === at && x.capability === cap; })[0];
    if (!a) return;
    uiSheet(a.capability,
      '<div class="ui-quiet">' + esc(a.actor) + " \u00b7 " + esc(stamp(a.at)) + " \u00b7 " + esc(a.tier) + " \u00b7 " + (a.dry_run ? "preview" : a.ok ? "applied" : "refused") + "</div>" +
      (a.message ? "<h3>Result</h3><div>" + esc(a.message) + "</div>" : "") +
      '<h3>Arguments</h3><pre class="log-block">' + esc(a.args || "{}") + "</pre>");
  }

  function loadActivity() {
    return request("/api/audit?limit=500").then(function (res) {
      if (res.ok) {
        state.audit = res.body.items || [];
        state.auditError = null;
        var pages = Math.max(1, Math.ceil(state.audit.length / PAGE_SIZE));
        if ((state.auditPage || 0) >= pages) state.auditPage = 0;
        return;
      }
      // A failure is not an empty history. Saying so keeps the two apart.
      state.audit = [];
      state.auditError = errorOf(res.body, "the activity log could not be read");
    });
  }

  // ---------------------------------------------------------------------- system

  function loadNotifications() {
    if (NOTIFY.loading || NOTIFY.data) return Promise.resolve();
    NOTIFY.loading = true;
    NOTIFY.error = "";
    return read("notify.list", {})
      .then(function (d) { NOTIFY.data = d; })
      .catch(function (e) { NOTIFY.error = e.message; })
      .then(function () {
        NOTIFY.loading = false;
        if (state.view === "system") render();
      });
  }

  function notificationsBody() {
    if (NOTIFY.error) return '<div class="notice notice-bad">' + esc(NOTIFY.error) + "</div>";
    if (!NOTIFY.data) return '<div class="loading"></div>';
    return opsNotify([NOTIFY.data]);
  }

  function renderSystem() {
    if (state.view !== "system") return;
    if (!NOTIFY.data && !NOTIFY.error && !NOTIFY.loading) loadNotifications();
    var checks = (state.doctor && state.doctor.checks) || [];
    var checkRows = checks
      .map(function (c) {
        var cls = c.level === "fail" ? "pill pill-bad" : c.level === "info" ? "pill pill-warn" : "pill pill-ok";
        return (
          "<tr>" +
          '<td class="cell-mono">' + esc(c.name) + "</td>" +
          '<td><span class="' + cls + '">' + esc(c.level) + "</span></td>" +
          "<td>" + esc(c.detail) + "</td>" +
          "</tr>"
        );
      })
      .join("");

    var s = state.session || { paths: {}, settings: {}, registry: { groups: [] } };
    var groups = (s.registry.groups || [])
      .map(function (g) {
        return '<span class="pill">' + esc(g.name) + " " + g.count + "</span>";
      })
      .join("");
    var reqs = (AI.requirements && AI.requirements.requirements) || [];
    var reqPanel = reqs.length
      ? '<div class="panel"><div class="panel-head"><span>Requirements</span><span class="panel-note">' +
        (AI.requirements.missing || 0) + " missing of " + reqs.length + "</span></div>" +
        '<div class="panel-body flush">' + reqs.map(requirementRow).join("") + "</div></div>"
      : "";

    el("main").innerHTML =
      '<div class="page">' +
      '<div class="page-head"><div>' + pageTitle("Diagnostics", "system") + '</div>' +
      '<div class="page-actions">' +
      '<button type="button" class="btn" data-action="refresh">Re-check</button>' +
      '<button type="button" class="btn btn-danger" data-action="quit">Quit devd</button>' +
      "</div></div>" +
      '<div class="ui-diag">' + healthBody(false) + "</div>" +
      (reqs.length && AI.requirements.missing ? '<details class="ui-fold" open><summary>Set up (' + AI.requirements.missing + " missing)</summary>" + reqPanel + "</details>" : "") +
      '<details class="ui-fold"><summary>Environment: ' + esc((AI.env && AI.env.mode) || "local") + '</summary><div class="panel"><div class="panel-head"><span>Environment</span><span class="panel-note">' + esc((AI.env && AI.env.mode) || "local") + '</span></div><div class="panel-body"><div class="row-sub">' + ((AI.env && AI.env.mode === "production") ? "Production mode is on. Agents need your approval for every change, plan approval cannot be skipped, and devd never deploys or opens ports by itself." : "Local mode. Agents work freely inside authorised project folders; anything destructive or system-wide still asks you. Turn production mode on for a machine that runs something real.") + '</div><div class="inline" style="margin-top:10px"><button type="button" class="btn ' + ((AI.env && AI.env.mode === "production") ? "" : "btn-danger") + '" data-action="env-toggle">' + ((AI.env && AI.env.mode === "production") ? "Switch to local (asks first)" : "Switch to production mode") + "</button></div></div></div></details>" +
      '<details class="ui-fold"><summary>Raw environment checks</summary><div class="panel"><div class="panel-head"><span>Raw environment checks</span>' +
      '<span class="panel-note">' + (state.doctor && state.doctor.ok ? "no failures" : "see levels") + "</span></div>" +
      '<div class="panel-body flush">' +
      '<table class="table"><thead><tr><th>Check</th><th>Level</th><th>Detail</th></tr></thead>' +
      "<tbody>" + checkRows + "</tbody></table>" +
      "</div></div></details>" +
      '<details class="ui-fold"><summary>Layout, settings and registry</summary><div class="panel"><div class="panel-head"><span>Layout</span></div>' +
      '<div class="panel-body"><dl class="kv">' +
      Object.keys(s.paths || {})
        .map(function (k) { return "<dt>" + esc(k) + "</dt><dd>" + esc(s.paths[k]) + "</dd>"; })
        .join("") +
      "</dl></div></div>" +
      '<div class="panel"><div class="panel-head"><span>Settings</span></div>' +
      '<div class="panel-body"><dl class="kv">' +
      Object.keys(s.settings || {})
        .map(function (k) { return "<dt>" + esc(k) + "</dt><dd>" + esc(JSON.stringify(s.settings[k])) + "</dd>"; })
        .join("") +
      "</dl></div></div>" +
      '<div class="panel"><div class="panel-head"><span>Registry</span>' +
      '<span class="panel-note">' + ((s.registry && s.registry.capabilities) || 0) + " capabilities</span></div>" +
      '<div class="panel-body inline">' + groups + "</div></div></details>" +
      '<details class="ui-fold"><summary>Notifications</summary>' + notificationsBody() + "</details>" +
      "</div>";
  }

  function loadSession() {
    return request("/api/session").then(function (res) {
      if (!res.ok) return;
      state.session = res.body;
      state.pending_approvals = res.body.pending_approvals || 0;
    });
  }

  function loadDoctor() {
    return request("/api/doctor").then(function (res) {
      if (!res.ok) return;
      state.doctor = res.body;
      state.health = state.doctor;
    });
  }

  function loadSystem() {
    return Promise.all([loadDoctor(), loadSession(), loadHealth(), read("setup.requirements").catch(function () { return null; }).then(function (r) { AI.requirements = r; })]);
  }


  // ------------------------------------------------------------ stacks: create

  var CREATE = {
    stacks: null, id: null, def: null, form: {}, advanced: false,
    plan: null, planError: null, timer: null, seq: 0,
    running: false, runId: null, log: [], logTimer: null, report: null,
    importPath: "", importCtx: null
  };

  function loadCreate() {
    if (CREATE.stacks) return Promise.resolve();
    return read("stacks.list").then(function (d) { CREATE.stacks = d || []; }).catch(function (e) {
      CREATE.stacks = [];
      toast("error", e.message);
    });
  }

  /* How many stacks devd can create a project from. The number comes from the
     server, because a number written in the interface is a number that will
     be wrong one day. */
  function stackCount() {
    if (CREATE.stacks && CREATE.stacks.length) return CREATE.stacks.length;
    return null;
  }

  function pickStack(id) {
    return read("stacks.describe", { stack: id }).then(function (d) {
      CREATE.id = id;
      CREATE.def = d.stack;
      CREATE.report = null;
      CREATE.log = [];
      var f = { name: "", destination: "", version: d.stack.versions.filter(function (v) { return v.default; })[0].id,
        package_manager: (d.stack.package_managers || [])[0] || "", git: true, install: true, extra: "", options: {} };
      (d.stack.options || []).forEach(function (o) { f.options[o.id] = o.default; });
      CREATE.form = f;
      scheduleValidate();
      renderCreate();
    }).catch(function (e) { toast("error", e.message); });
  }

  function createRequest() {
    var f = CREATE.form;
    var req = { stack: CREATE.id, name: f.name, version: f.version, git: f.git, install: f.install, options: f.options };
    if (f.package_manager) req.package_manager = f.package_manager;
    if (f.destination) req.destination = f.destination;
    var extra = f.extra.trim() ? f.extra.trim().split(/\s+/) : [];
    if (extra.length) req.extra_args = extra;
    return req;
  }

  function scheduleValidate() {
    clearTimeout(CREATE.timer);
    var seq = ++CREATE.seq;
    CREATE.timer = setTimeout(function () {
      if (!CREATE.id) return;
      read("stacks.validate", createRequest()).then(function (p) {
        if (seq !== CREATE.seq) return;
        CREATE.plan = p; CREATE.planError = null; paintReview();
      }).catch(function (e) {
        if (seq !== CREATE.seq) return;
        CREATE.plan = null; CREATE.planError = e.message; paintReview();
      });
    }, 350);
  }

  function pill(kind, text) { return '<span class="pill pill-' + kind + '">' + esc(text) + "</span>"; }

  function docLinks(docs) {
    return (docs || []).map(function (d) {
      return '<a class="link" href="' + attr(d.url) + '" target="_blank" rel="noopener noreferrer">' + esc(d.title) + "</a>";
    }).join(" · ");
  }

  function renderCreate() {
    if (state.view !== "create") return;
    if (!CREATE.stacks) return renderLoading();
    if (!CREATE.id) {
      var cats = {};
      CREATE.stacks.filter(function (s) { return s.creatable; }).forEach(function (s) {
        (cats[s.category] = cats[s.category] || []).push(s);
      });
      var order = ["web", "backend", "frontend", "mobile", "language"];
      var html = order.filter(function (c) { return cats[c]; }).map(function (c) {
        return uiHead(c.charAt(0).toUpperCase() + c.slice(1), cats[c].length) + '<div class="ui-tiles">' +
          cats[c].map(function (s) {
            return '<button type="button" class="ui-tile" data-action="create-pick" data-id="' + attr(s.id) + '"><b>' + brandImg(s.id, "brand-dot") + esc(s.name) + '</b><span>' + esc(s.summary) + '</span></button>';
          }).join("") + "</div>";
      }).join("");
      el("main").innerHTML = page("New project", "",
        '<button type="button" class="btn" data-action="nav" data-view="develop">Import an existing project</button>', html);
      return;
    }
    var d = CREATE.def, f = CREATE.form;
    var fld = function (label, help, control) {
      return '<div class="field"><label class="field-label">' + esc(label) + "</label>" + control +
        (help ? '<div class="row-sub">' + esc(help) + "</div>" : "") + "</div>";
    };
    var sel = function (key, items, value, isOpt) {
      return '<select class="input" data-create="' + attr(key) + '"' + (isOpt ? ' data-opt="1"' : "") + ">" +
        items.map(function (i) { return '<option value="' + attr(i.value) + '"' + (i.value === value ? " selected" : "") + ">" + esc(i.label) + "</option>"; }).join("") + "</select>";
    };
    var fields = [];
    fields.push(fld("Project name", d.name_hint, '<input class="input mono" data-create="name" value="' + attr(f.name) + '" placeholder="my-app" spellcheck="false">'));
    fields.push(fld("Create in", "Leave blank for the projects root.", '<input class="input mono" data-create="destination" value="' + attr(f.destination) + '" placeholder="projects root" spellcheck="false">'));
    if (d.versions.length > 1) fields.push(fld("Version", "", sel("version", d.versions.map(function (v) { return { value: v.id, label: v.label + (v.default ? " (default)" : "") }; }), f.version)));
    if ((d.package_managers || []).length > 1) fields.push(fld("Package manager", "", sel("package_manager", d.package_managers.map(function (p) { return { value: p, label: p }; }), f.package_manager)));
    (d.options || []).forEach(function (o) {
      if (o.advanced && !CREATE.advanced) return;
      var v = f.options[o.id], ctl = "";
      var flag = o.flag ? " (" + o.flag + ")" : "";
      if (o.kind === "bool") ctl = '<label class="check"><input type="checkbox" data-create="' + attr(o.id) + '" data-opt="1"' + (v ? " checked" : "") + "> Enabled</label>";
      else if (o.kind === "select") ctl = sel(o.id, o.choices.map(function (c) { return { value: c.value, label: c.label }; }), v, true);
      else if (o.kind === "multi") ctl = o.choices.map(function (c) {
        return '<label class="check"><input type="checkbox" data-create="' + attr(o.id) + '" data-multi="' + attr(c.value) + '" data-opt="1"' + ((v || []).indexOf(c.value) >= 0 ? " checked" : "") + "> " + esc(c.label) + "</label>";
      }).join(" ");
      else ctl = '<input class="input mono" data-create="' + attr(o.id) + '" data-opt="1" value="' + attr(v) + '" spellcheck="false">';
      fields.push(fld(o.label + flag, o.help, ctl));
    });
    fields.push(fld("Git", "", '<label class="check"><input type="checkbox" data-create="git"' + (f.git ? " checked" : "") + "> Initialise a repository</label>"));
    if (d.init.install_toggle) fields.push(fld("Dependencies", "", '<label class="check"><input type="checkbox" data-create="install"' + (f.install ? " checked" : "") + "> Install now</label>"));
    var hasAdv = (d.options || []).some(function (o) { return o.advanced; });
    var adv = '<div class="field"><label class="check"><input type="checkbox" data-create="__advanced"' + (CREATE.advanced ? " checked" : "") + "> Show advanced options</label></div>";
    if (CREATE.advanced) fields.push(fld("Extra arguments", "Appended exactly as typed to the scaffolding command shown in the review.", '<input class="input mono" data-create="extra" value="' + attr(f.extra) + '" placeholder="--flag value" spellcheck="false">'));

    var form = '<div class="panel"><div class="panel-head"><span>' + esc(d.name) + '</span><span class="panel-note">' + docLinks(d.docs) + "</span></div>" +
      '<div class="panel-body">' + '<div class="row-sub" style="margin-bottom:12px">' + esc(d.summary) + "</div>" + fields.join("") + (hasAdv || true ? adv : "") + "</div></div>";
    var review = '<div class="panel" id="create-review"></div>';
    var progress = '<div id="create-progress"></div>';
    el("main").innerHTML = page("Create " + d.name + " project", d.description || d.summary,
      '<button type="button" class="btn" data-action="create-back">Back to stacks</button>',
      '<div class="two-col"><div>' + form + "</div><div>" + review + progress + "</div></div>");
    paintReview();
    paintProgress();
  }

  function paintReview() {
    var box = el("create-review");
    if (!box) return;
    if (CREATE.planError) { box.innerHTML = '<div class="panel-head"><span>Review</span></div><div class="panel-body notice notice-bad">' + esc(CREATE.planError) + "</div>"; return; }
    var p = CREATE.plan;
    if (!p) { box.innerHTML = '<div class="panel-head"><span>Review</span></div><div class="panel-body dim">Checking…</div>'; return; }
    var h = '<div class="panel-head"><span>Review</span><span class="panel-note">' + (p.valid ? (p.ready ? pill("ok", "ready") : pill("warn", "prerequisites missing")) : pill("bad", "needs attention")) + "</span></div><div class=\"panel-body\">";
    (p.issues || []).forEach(function (i) { h += '<div class="notice notice-bad"><b>' + esc(i.field) + "</b>: " + esc(i.message) + "</div>"; });
    if (p.valid) {
      h += '<div class="kv"><dt>Directory</dt><dd class="mono">' + esc(p.project_dir) + "</dd></div>";
      h += '<div class="field-label" style="margin-top:12px">Command devd will run</div><pre class="log-block">' + esc(p.command) + "</pre>";
      if ((p.steps || []).length > 1) h += '<div class="row-sub">' + p.steps.map(function (s) { return esc(s.title); }).join(" → ") + "</div>";
    }
    h += '<div class="field-label" style="margin-top:12px">Prerequisites</div>';
    (p.prerequisites || []).forEach(function (q) {
      var ok = q.status === "ok" || q.status === "unknown_version";
      h += '<div class="inline" style="margin:4px 0">' + pill(ok ? "ok" : "bad", ok ? "ok" : q.status.replace("_", " ")) + "<span>" + esc(q.name) + " " + esc(q.found || "") + (q.min ? ' <span class="dim">needs ≥ ' + esc(q.min) + "</span>" : "") + "</span>" +
        (!ok && q.status === "missing" ? '<button type="button" class="btn btn-mini" data-action="tool-install" data-id="' + attr(q.tool) + '">Install</button>' : "") + "</div>" +
        (!ok && q.fix ? '<div class="row-sub">' + esc(q.fix) + "</div>" : "");
    });
    (p.services || []).forEach(function (sv) { h += '<div class="row-sub">Needs ' + esc(sv.tool) + " (" + esc(sv.why) + "): " + (sv.running ? "running" : sv.installed ? "installed, not running" : "not installed") + "</div>"; });
    (p.warnings || []).forEach(function (w) { h += '<div class="notice notice-warn">' + esc(w) + "</div>"; });
    h += '<div class="inline" style="margin-top:14px"><button type="button" class="btn btn-go" data-action="create-run"' + (p.valid && p.ready && !CREATE.running ? "" : " disabled") + ">Create project</button></div>";
    h += "</div>";
    box.innerHTML = h;
  }

  function paintProgress() {
    var box = el("create-progress");
    if (!box) return;
    var h = "";
    if (CREATE.running || CREATE.log.length) {
      h += '<div class="panel"><div class="panel-head"><span>Progress</span><span class="panel-note">' +
        (CREATE.running ? '<button type="button" class="btn btn-mini btn-danger" data-action="create-cancel">Cancel</button>' : "") + "</span></div>" +
        '<div class="panel-body"><pre class="log-block">' + esc(CREATE.log.slice(-60).join("\n")) + "</pre></div></div>";
    }
    var r = CREATE.report;
    if (r) {
      var kind = r.status === "ok" ? "ok" : r.status === "partial" ? "warn" : "bad";
      h += '<div class="panel"><div class="panel-head"><span>Result</span><span class="panel-note">' + pill(kind, r.status) + "</span></div><div class=\"panel-body\">";
      if (r.error) h += '<div class="notice notice-' + (r.status === "partial" ? "warn" : "bad") + '">' + esc(r.error) + "</div>";
      (r.steps || []).forEach(function (st) { h += '<div class="inline">' + pill(st.status === "ok" || st.status === "skipped" ? "ok" : st.status === "warning" ? "warn" : "bad", st.status) + "<span>" + esc(st.title) + "</span></div>" + (st.status === "failed" ? '<pre class="log-block">' + esc(st.detail) + "</pre>" : ""); });
      (r.checks || []).forEach(function (c) { h += '<div class="inline">' + pill(c.status === "ok" ? "ok" : c.status === "skipped" ? "" : "bad", c.status) + "<span>" + esc(c.title) + "</span></div>"; });
      (r.next || []).forEach(function (n) { h += '<div class="row-sub">' + esc(n) + "</div>"; });
      if (r.status === "ok" || r.status === "partial") h += '<div class="inline" style="margin-top:12px"><button type="button" class="btn btn-primary" data-action="open-develop" data-path="' + attr(r.project_dir) + '">Open in Develop</button></div>';
      h += "</div></div>";
    }
    box.innerHTML = h;
  }

  function pollCreateLog() {
    clearInterval(CREATE.logTimer);
    CREATE.logTimer = setInterval(function () {
      if (!CREATE.running) return clearInterval(CREATE.logTimer);
      read("stacks.log", { run_id: CREATE.runId, lines: 80 }).then(function (d) { CREATE.log = d.lines || []; paintProgress(); }).catch(function () {});
    }, 800);
  }

  // ------------------------------------------------------------ stacks: develop

  var DEV = { projects: null, path: null, ctx: null, busy: false, procs: [], log: null, importPath: "", detect: null, error: null };

  function loadDevelop() {
    return Promise.all([read("project.list").catch(function () { return []; }), read("task.list").catch(function () { return []; })]).then(function (r) {
      DEV.projects = r[0] || []; DEV.procs = r[1] || [];
    }).then(function () { return DEV.path && !DEV.ctx ? detectProject(DEV.path, true) : null; });
  }

  function detectProject(path, quiet) {
    DEV.path = path; DEV.busy = true; DEV.error = null;
    return read("project.detect", { path: path }).then(function (c) { DEV.ctx = c; DEV.busy = false; if (!quiet) renderDevelop(); })
      .catch(function (e) { DEV.busy = false; DEV.ctx = null; DEV.error = e.message; if (!quiet) renderDevelop(); });
  }

  function renderDevelop() {
    if (state.view !== "develop") return;
    if (!DEV.projects) return renderLoading();
    var list = DEV.projects.map(function (p) {
      return '<button type="button" class="svc ' + (DEV.path === p.path ? "active" : "") + '" data-action="dev-open" data-path="' + attr(p.path) + '" style="width:100%;text-align:left">' +
        '<div class="svc-main"><div class="svc-name">' + esc(p.name) + '</div><div class="svc-sub mono nowrap">' + esc(p.stack || "not detected yet") + " · " + esc(p.path) + "</div></div></button>";
    }).join("") || '<div class="panel-body empty-inline">No projects yet. Create one or import a checkout.</div>';
    var imp = '<div class="panel"><div class="panel-head"><span>Import an existing project</span></div><div class="panel-body">' +
      '<div class="inline"><input class="input mono grow" id="dev-import" placeholder="Where the project is" value="' + attr(DEV.importPath) + '" spellcheck="false">' +
      '<button type="button" class="btn" data-action="dev-pick">Choose folder…</button>' +
      '<button type="button" class="btn" data-action="dev-detect">Detect</button><button type="button" class="btn btn-primary" data-action="dev-import">Import</button></div>' +
      '<div class="row-sub">Nothing is changed in the project. Clone a repository first with <span class="mono">git clone</span> or the Projects view.</div></div></div>';
    el("main").innerHTML = page("Stacks and tasks", "Detect any project's stack, run its tasks, and make it available to agents.", "",
      '<div class="two-col dev-split"><div>' + uiHead("Projects", DEV.projects.length) + '<div class="dev-list">' + list + "</div>" + imp + procPanel() + "</div><div>" + devDetail() + "</div></div>");
  }

  function procPanel() {
    if (!DEV.procs.length) return "";
    return '<div class="panel"><div class="panel-head"><span>Running tasks</span></div>' + DEV.procs.map(function (p) {
      return '<div class="svc"><div class="svc-main"><div class="svc-name">' + dot(p.running) + " " + esc(p.name) + '</div><div class="svc-sub mono">pid ' + esc(p.pid) + (p.running ? "" : " · exited " + esc(p.exit_code)) + "</div></div>" +
        '<div class="svc-actions"><button type="button" class="btn btn-mini" data-action="proc-log" data-name="' + attr(p.name) + '">Log</button>' +
        '<button type="button" class="btn btn-mini btn-danger" data-action="proc-stop" data-name="' + attr(p.name) + '">Stop</button></div></div>';
    }).join("") + "</div>";
  }

  /* A project can ask for one runtime in several places (package.json engines,
     .nvmrc, a lockfile). That is one requirement with evidence, not three
     unrelated ones. Group by the tool and keep every source. */
  function runtimeNeeds(list) {
    var order = [], by = {};
    (list || []).forEach(function (r) {
      var tool = r.tool || "runtime";
      if (!by[tool]) { by[tool] = { tool: tool, wants: [], installed: {}, satisfied: null, sources: {} }; order.push(tool); }
      var g = by[tool];
      var want = String(r.constraint || "").trim();
      if (want && g.wants.indexOf(want) === -1) g.wants.push(want);
      if (r.installed) g.installed[r.installed] = true;
      if (r.source) g.sources[r.source] = true;
      if (r.satisfied === false) g.satisfied = false;
      else if (r.satisfied === true && g.satisfied !== false) g.satisfied = true;
    });
    return order.map(function (k) { return by[k]; });
  }

  function missingNeeds(list) {
    var order = [], by = {};
    (list || []).forEach(function (m) {
      var tool = m.tool || "tool";
      if (!by[tool]) { by[tool] = { tool: tool, why: [] }; order.push(tool); }
      if (m.why && by[tool].why.indexOf(m.why) === -1) by[tool].why.push(m.why);
    });
    return order.map(function (k) { return by[k]; });
  }

  function devDetail() {
    if (DEV.busy) return '<div class="panel"><div class="panel-body dim">Inspecting…</div></div>';
    if (DEV.error) return '<div class="panel"><div class="panel-body notice notice-bad">' + esc(DEV.error) + "</div></div>";
    var c = DEV.detect || DEV.ctx;
    if (!c) return uiEmpty("Choose a project", "devd detects its stack, package manager and runtimes, lists the tasks it can run, and shows what an agent may run there. Detecting reads files; it changes nothing.");
    var h = '<div class="panel"><div class="panel-head"><span>' + esc(c.name) + '</span><span class="panel-note mono">' + esc(c.path) + '</span></div><div class="panel-body">';
    if (c.ambiguous) h += '<div class="notice notice-warn">' + esc(c.ambiguity) + "</div>";
    h += '<div class="kv"><dt>Stack</dt><dd>' + (c.stacks.length ? c.stacks.map(function (s, i) { return (i === 0 ? "<b>" : "") + esc(s.name) + " " + pill(s.confidence === "high" ? "ok" : "warn", s.confidence) + (i === 0 ? "</b>" : ""); }).join(" · ") : "unknown") + "</dd></div>";
    if (c.stacks.length) h += '<div class="row-sub">' + esc(c.stacks[0].evidence.join("; ")) + "</div>";
    h += '<div class="kv"><dt>Package manager</dt><dd>' + esc(c.package_manager || "-") + ' <span class="dim">' + esc(c.package_manager_evidence || "") + "</span></dd></div>";
    var needs = runtimeNeeds(c.runtimes || []);
    var needTools = {};
    needs.forEach(function (g) { needTools[g.tool] = true; });
    needs.forEach(function (g) {
      var installed = Object.keys(g.installed).join(", ") || "none";
      var wants = g.wants.join(" / ") || "any version";
      var state = g.satisfied === true ? pill("ok", "satisfied") : g.satisfied === false ? pill("bad", "not satisfied") : pill("warn", "not checked");
      h += '<div class="kv"><dt>' + esc(g.tool) + '</dt><dd>wants ' + esc(wants) + ' <span class="dim">(' + esc(Object.keys(g.sources).join(", ")) + ")</span> · installed " + esc(installed) + " " + state +
        (g.satisfied === false ? ' <button type="button" class="btn btn-mini" data-action="nav" data-view="runtimes">Runtimes</button>' : "") + "</dd></div>";
    });
    missingNeeds(c.missing || []).forEach(function (m) {
      h += '<div class="notice notice-warn"><span class="grow"><b>' + esc(m.tool) + "</b>: " + esc(m.why.join("; ")) + "</span>" +
        (needTools[m.tool] ? "" : '<button type="button" class="btn btn-mini" data-action="tool-install" data-id="' + attr(m.tool) + '">Install</button>') + "</div>";
    });
    (c.services || []).forEach(function (sv) { h += '<div class="kv"><dt>Service</dt><dd>' + esc(sv.tool) + " " + (sv.running ? pill("ok", "running") : sv.installed ? pill("warn", "stopped") : pill("bad", "missing")) + ' <span class="dim">' + esc(sv.why) + "</span></dd></div>"; });
    if (c.env && c.env.keys.length) h += '<div class="kv"><dt>Environment</dt><dd>' + c.env.keys.length + " variables in " + esc(c.env.example_files.join(", ")) + (c.env.unset_keys.length ? "; unset: " + esc(c.env.unset_keys.join(", ")) : "") + ' <span class="dim">(values are never shown)</span></dd></div>';
    if (c.docs.length) h += '<div class="kv"><dt>Docs</dt><dd class="mono">' + esc(c.docs.join(", ")) + "</dd></div>";
    if (c.official_docs.length) h += '<div class="kv"><dt>Official docs</dt><dd>' + docLinks(c.official_docs) + "</dd></div>";
    h += "</div></div>";
    h += '<div class="panel"><div class="panel-head"><span>Tasks</span><span class="panel-note">' + c.tasks.length + "</span></div><div class=\"panel-body flush\"><table class=\"table\"><tbody>" +
      c.tasks.map(function (t) {
        return "<tr><td class=\"cell-strong\">" + esc(t.id) + '</td><td class="cell-mono">' + esc(t.command) + "</td><td>" + pill(t.risk === "safe" ? "ok" : t.risk === "mutating" ? "warn" : "bad", t.risk) + "</td><td>" +
          (t.available ? '<button type="button" class="btn btn-mini' + (t.risk === "destructive" ? " btn-danger" : " btn-primary") + '" data-action="task-run" data-task="' + attr(t.id) + '" data-risk="' + attr(t.risk) + '">' + (t.long_running ? "Start" : "Run") + "</button>" : '<span class="dim" title="' + attr(t.unavailable_reason) + '">unavailable</span>') + "</td></tr>";
      }).join("") + "</tbody></table></div></div>";
    if (DEV.log) h += '<div class="panel"><div class="panel-head"><span>Output</span></div><div class="panel-body"><pre class="log-block">' + esc(DEV.log) + "</pre></div></div>";
    return h;
  }

  // -------------------------------------------------------------- toolchain

  var TC = { data: null, devices: null, loading: false };
  var TOOL_RUNTIME_IDS = { php: 1, node: 1, python: 1, ruby: 1, go: 1 };
  var TOOL_CATEGORY = { vcs: "Version control", "package-manager": "Package managers", runtime: "Runtimes", build: "Build tools", mobile: "Mobile", framework: "Frameworks", editor: "Editors", browser: "Browsers", design: "Design", container: "Containers", database: "Databases", ai: "AI tools" };

  // Runtimes, services and databases have their own screens. Keeping them out
  // of Tools means each install, start and version appears in one place.
  function toolOwnedElsewhere(t) {
    if (!t || !t.tool) return true;
    if (t.tool.category === "database" || t.tool.category === "container") return true;
    if (t.tool.id === "nginx") return true;
    return !!TOOL_RUNTIME_IDS[t.tool.id];
  }

  /* Rails is a framework command, not a package manager. The catalogue's
     category is metadata, but the Tools screen groups by what the tool is for. */
  function toolCategory(t) {
    if (t && t.tool && t.tool.id === "rails") return "framework";
    return (t && t.tool && t.tool.category) || "utility";
  }

  function loadToolchain() {
    TC.loading = true;
    if (state.view === "toolchain") renderToolchain();
    return read("toolchain.list")
      .then(function (d) { TC.data = d; })
      .catch(function (e) { toast("error", e.message); })
      .then(function () { TC.loading = false; });
  }

  function renderToolchain() {
    if (state.view !== "toolchain") return;
    var actions = TC.loading
      ? '<span class="ui-checking"><span class="spin" aria-hidden="true"></span>Checking…</span>'
      : '<button type="button" class="btn" data-action="refresh">Re-check</button>';
    if (!TC.data) {
      el("main").innerHTML = page("Tools", "", "", '<div class="loading" role="status" aria-live="polite">Loading…</div><div class="loading-note">devd runs each tool it knows to see what is installed. That takes a moment.</div>');
      return;
    }
    var host = TC.data.host, tools = TC.data.tools;
    var q = state.filters.toolchain || "";
    var cats = {};
    tools.filter(function (t) { return !toolOwnedElsewhere(t) && keeps(q, t.tool.id, t.tool.name, t.tool.purpose, t.tool.category); })
      .forEach(function (t) { var c = toolCategory(t); (cats[c] = cats[c] || []).push(t); });
    var notes = [];
    if (!host.root && !host.sudo && host.os === "linux") notes.push("devd has neither root nor passwordless sudo, so package installs show the exact command to run yourself.");
    if (!host.managers.length) notes.push("No supported package manager was found; installs will show manual steps.");
    var html = '<div class="ui-more">The command-line tools and package managers projects use. Runtimes, services and databases each have their own tab.</div>' +
      notes.map(function (n) { return uiRow({ accent: true, dot: "warn", title: esc(n) }); }).join("") +
      '<div class="filter-row">' + filterBox("toolchain", "Search tools") + "</div>";
    if (!Object.keys(cats).length) html += uiEmpty('No tool matches "' + q + '"');
    Object.keys(cats).forEach(function (c) {
      var list = cats[c].slice().sort(function (a, b) { return (a.status.installed ? 1 : 0) - (b.status.installed ? 1 : 0); });
      html += uiHead(TOOL_CATEGORY[c] || (c.charAt(0).toUpperCase() + c.slice(1)), list.length) + uiList(list.map(function (t) {
        var st = t.status, sv = t.service, id = t.tool.id;
        var acts = "";
        if (!st.installed && !t.tool.per_project) acts += '<button type="button" class="btn btn-mini btn-primary" data-action="tool-install" data-id="' + attr(id) + '">Install</button>';
        if (sv && st.installed) {
          acts += sv.healthy ? '<button type="button" class="btn btn-mini" data-action="svc" data-act="stop" data-id="' + attr(id) + '">Stop</button><button type="button" class="btn btn-mini btn-ghost" data-action="svc" data-act="restart" data-id="' + attr(id) + '">Restart</button>' : '<button type="button" class="btn btn-mini btn-primary" data-action="svc" data-act="start" data-id="' + attr(id) + '">Start</button>';
          acts += '<button type="button" class="btn btn-mini btn-ghost" data-action="svc-logs" data-id="' + attr(id) + '">Logs</button>';
        }
        if (id === "adb" && st.installed) acts += '<button type="button" class="btn btn-mini btn-ghost" data-action="devices">Devices</button>';
        if (id === "eas-cli" && st.installed) acts += '<button type="button" class="btn btn-mini btn-ghost" data-action="eas-projects">Projects</button>';
        if (st.installed && !t.tool.per_project) acts += '<button type="button" class="btn btn-mini btn-ghost" data-action="tool-remove" data-id="' + attr(id) + '">Remove</button>';
        var sub = st.installed ? (sv ? sv.detail + (sv.port_conflict ? " - port held by " + sv.port_conflict : "") : t.tool.purpose) : (st.problem && !t.tool.per_project ? st.problem : t.tool.purpose);
        return uiRow({
          dot: st.installed ? (st.outdated ? "warn" : "ok") : (t.tool.per_project ? "off" : "bad"),
          title: brandImg(id, "brand-dot") + esc(t.tool.name) + (st.version ? ' <span class="ui-quiet" style="display:inline;padding:0">' + esc(st.version) + "</span>" : "") + (t.tool.per_project ? ' <span class="pill">per project</span>' : "") + (st.outdated ? ' <span class="pill pill-warn">outdated</span>' : ""),
          sub: esc(uiSummary(sub, 140)),
          actions: acts, hover: st.installed
        });
      }));
    });
    el("main").innerHTML = page("Tools", "", actions, html);
  }


  // ============================================================ assistant era
  // Home, the assistant, the inbox, providers and the health indicator. They
  // only call capabilities; every decision is made by the registry.

  function ago(iso) {
    var t = Date.parse(iso);
    if (!t) return "";
    return relativeTime(Math.floor(t / 1000));
  }

  /* The time of day, for the line under a message. */
  function shortTime(value) {
    if (!value) return "";
    var d = new Date(value);
    if (isNaN(d.getTime())) return "";
    return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }

  /* A timestamp a person can scan: the clock time for today, the date and the
     time for anything older. The full value stays in the title. */
  function stamp(iso) {
    var t = Date.parse(iso);
    if (!t) return iso || "";
    var d = new Date(t), now = new Date();
    var time = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
    if (d.toDateString() === now.toDateString()) return time;
    return d.toLocaleDateString([], { month: "short", day: "numeric" }) + " " +
      d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }

  /* A small, safe Markdown renderer: code fences, inline code, bold, lists, links. */
  // ---------------------------------------------------------------- markdown
  //
  // Text from a model, a report or a note is Markdown. Everything is escaped before it is
  // turned into markup, so nothing in the text can add a tag; links are only http, https and
  // mailto; images are shown as links so a remote file is never fetched on the page's behalf.

  function mdLink(url, label) {
    return '<a class="link" href="' + url + '" target="_blank" rel="noopener noreferrer">' + label + "</a>";
  }

  function mdInline(raw) {
    var codes = [];
    var s = String(raw).replace(/`([^`\n]+)`/g, function (_, c) { codes.push(c); return "\u0000" + (codes.length - 1) + "\u0000"; });
    s = esc(s);
    s = s.replace(/!\[([^\]\n]*)\]\((https?:\/\/[^)\s]+)\)/g, function (_, alt, url) { return mdLink(url, alt || "image"); });
    s = s.replace(/\[([^\]\n]+)\]\(((?:https?:\/\/|mailto:)[^)\s]+)\)/g, function (_, label, url) { return mdLink(url, label); });
    s = s.replace(/(^|[\s(])(https?:\/\/[^\s<)]*[^\s<).,;:!?])/g, function (_, pre, url) { return pre + mdLink(url, url); });
    s = s.replace(/\*\*(?=\S)([^\n]*?\S)\*\*/g, "<strong>$1</strong>");
    s = s.replace(/(^|[^\w])__(?=\S)([^\n]*?\S)__(?!\w)/g, "$1<strong>$2</strong>");
    s = s.replace(/~~(?=\S)([^\n]*?\S)~~/g, "<del>$1</del>");
    s = s.replace(/(^|[^*\w])\*(?=[^\s*])([^*\n]*?[^\s*])\*(?!\w)/g, "$1<em>$2</em>");
    s = s.replace(/(^|[^\w])_(?=[^\s_])([^_\n]*?[^\s_])_(?!\w)/g, "$1<em>$2</em>");
    return s.replace(/\u0000(\d+)\u0000/g, function (_, n) { return "<code>" + esc(codes[+n]) + "</code>"; });
  }

  var MD_ITEM = /^(\s*)([-*+]|\d{1,9}[.)])\s+(.*)$/;
  var MD_FENCE = /^\s{0,3}(```+|~~~+)\s*([\w+#.-]*)[^\n]*$/;
  var MD_HR = /^\s{0,3}([-*_])(\s*\1){2,}\s*$/;
  var MD_HEAD = /^\s{0,3}(#{1,6})\s+(.*?)\s*#*\s*$/;
  var MD_TABLE_SEP = /^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$/;

  function mdCells(line) {
    var t = line.trim();
    if (t.charAt(0) === "|") t = t.slice(1);
    if (t.charAt(t.length - 1) === "|" && t.charAt(t.length - 2) !== "\\") t = t.slice(0, -1);
    return t.split(/(?<!\\)\|/).map(function (c) { return c.replace(/\\\|/g, "|").trim(); });
  }

  function mdStartsBlock(lines, i) {
    var l = lines[i];
    return MD_FENCE.test(l) || MD_HEAD.test(l) || MD_HR.test(l) || /^\s{0,3}>/.test(l) || MD_ITEM.test(l) ||
      (l.indexOf("|") >= 0 && i + 1 < lines.length && MD_TABLE_SEP.test(lines[i + 1]) && lines[i + 1].indexOf("-") >= 0);
  }

  function mdList(lines, start) {
    var first = MD_ITEM.exec(lines[start]);
    var base = first[1].length, ordered = /\d/.test(first[2].charAt(0));
    var items = [], i = start;
    while (i < lines.length) {
      var m = MD_ITEM.exec(lines[i]);
      if (!m) {
        if (lines[i].trim() === "") {
          var j = i + 1;
          while (j < lines.length && lines[j].trim() === "") j++;
          var nm = j < lines.length ? MD_ITEM.exec(lines[j]) : null;
          if (nm && nm[1].length >= base) { i = j; continue; }
          break;
        }
        if (items.length && /^\s+\S/.test(lines[i]) && lines[i].match(/^\s*/)[0].length > base) {
          items[items.length - 1].text += "\n" + lines[i].trim();
          i++;
          continue;
        }
        break;
      }
      var ind = m[1].length;
      if (ind < base) break;
      // A different kind of list at the same level is a new list.
      if (ind === base && /\d/.test(m[2].charAt(0)) !== ordered) break;
      if (ind > base && items.length) {
        var sub = mdList(lines, i);
        items[items.length - 1].children += sub.html;
        i = sub.next;
        continue;
      }
      items.push({ text: m[3], children: "" });
      i++;
    }
    var startNum = ordered ? parseInt(first[2], 10) : 1;
    var html = "<" + (ordered ? "ol" : "ul") + (ordered && startNum !== 1 ? ' start="' + startNum + '"' : "") + ">" +
      items.map(function (it) {
        var task = /^\[( |x|X)\]\s+([\s\S]*)$/.exec(it.text);
        var body = task ? it.text.slice(4) : it.text;
        return "<li" + (task ? ' class="md-task"' : "") + ">" +
          (task ? '<span class="md-check' + (task[1] === " " ? "" : " on") + '" aria-hidden="true"></span>' : "") +
          mdInline(body).replace(/\n/g, "<br>") + it.children + "</li>";
      }).join("") + "</" + (ordered ? "ol" : "ul") + ">";
    return { html: html, next: i };
  }

  function mdBlocks(lines) {
    var out = [], i = 0;
    while (i < lines.length) {
      var line = lines[i], m;
      if (line.trim() === "") { i++; continue; }
      if ((m = MD_FENCE.exec(line))) {
        var fence = m[1], lang = m[2], body = [];
        i++;
        while (i < lines.length && !(new RegExp("^\\s{0,3}" + fence.charAt(0) + "{" + fence.length + ",}\\s*$").test(lines[i]))) { body.push(lines[i]); i++; }
        i++;
        out.push('<div class="md-code"><div class="md-code-head"><span>' + esc(lang || "text") + '</span><button type="button" class="md-copy" data-action="copy-code">Copy</button></div><pre class="code"><code>' + esc(body.join("\n")) + "</code></pre></div>");
        continue;
      }
      if ((m = MD_HEAD.exec(line))) { out.push('<h' + m[1].length + ' class="md-h">' + mdInline(m[2]) + "</h" + m[1].length + ">"); i++; continue; }
      if (MD_HR.test(line)) { out.push('<hr class="md-hr">'); i++; continue; }
      if (/^\s{0,3}>/.test(line)) {
        var quote = [];
        while (i < lines.length && /^\s{0,3}>/.test(lines[i])) { quote.push(lines[i].replace(/^\s{0,3}>\s?/, "")); i++; }
        out.push('<blockquote class="md-quote">' + mdBlocks(quote) + "</blockquote>");
        continue;
      }
      if (line.indexOf("|") >= 0 && i + 1 < lines.length && MD_TABLE_SEP.test(lines[i + 1]) && lines[i + 1].indexOf("-") >= 0) {
        var head = mdCells(line), aligns = mdCells(lines[i + 1]).map(function (c) {
          return /^:-+:$/.test(c) ? "center" : /^-+:$/.test(c) ? "right" : /^:-+$/.test(c) ? "left" : "";
        });
        i += 2;
        var rows = [];
        while (i < lines.length && lines[i].trim() !== "" && lines[i].indexOf("|") >= 0) { rows.push(mdCells(lines[i])); i++; }
        var cell = function (tag, c, k) { return "<" + tag + (aligns[k] ? ' style="text-align:' + aligns[k] + '"' : "") + ">" + mdInline(c) + "</" + tag + ">"; };
        out.push('<div class="md-table"><table><thead><tr>' + head.map(function (c, k) { return cell("th", c, k); }).join("") + "</tr></thead><tbody>" +
          rows.map(function (r) { return "<tr>" + head.map(function (_, k) { return cell("td", r[k] || "", k); }).join("") + "</tr>"; }).join("") + "</tbody></table></div>");
        continue;
      }
      if (MD_ITEM.test(line)) { var list = mdList(lines, i); out.push(list.html); i = list.next; continue; }
      var para = [line];
      i++;
      while (i < lines.length && lines[i].trim() !== "" && !mdStartsBlock(lines, i)) { para.push(lines[i]); i++; }
      out.push("<p>" + para.map(function (l) { return mdInline(l.trim()); }).join("<br>") + "</p>");
    }
    return out.join("");
  }

  function md(text) {
    var html = mdBlocks(String(text || "").replace(/\r\n?/g, "\n").split("\n"));
    return html ? '<div class="md">' + html + "</div>" : "";
  }

  var AI = {
    health: null, healthOpen: false,
    counts: { approvals: 0, questions: 0 },
    home: null, askText: "", askProject: "",
    providers: null, presets: null, catalogue: null, provForm: null, provBusy: {}, localFound: null,
    live: null, streamTail: "", streamPainted: "", streamFrame: 0, streamThread: "",
    threads: null, thread: null, events: [], pending: [], running: false, lastId: 0, queued: 0,
    run: null,
    pickProvider: "", pickModel: "",
    composer: "", project: "", timer: null, openSteps: {}, todos: [], todosAt: 0,
    inbox: null, requests: null, history: null
  };

  /* A run is not another destination. It is the durable record behind a
   * conversation or a team run, and it is opened from that work, in place, as a
   * sheet. The sheet is a snapshot: it says what was recorded, not what the
   * live stream happened to deliver. */
  var RUN = { id: null, data: null, events: [], hasOlder: false, error: null, changes: null };

  var RUN_STATE_WORD = {
    intake: "intake", planning: "planning", awaiting_approval: "needs your decision",
    executing: "working", verifying: "verifying", review: "ready for review",
    completed: "completed", unverified: "not verified", paused: "paused",
    blocked: "blocked", failed: "failed", cancelled: "cancelled"
  };

  function runPill(state) {
    var cls = state === "failed" || state === "blocked" ? "pill-bad"
      : state === "unverified" || state === "awaiting_approval" || state === "paused" || state === "review" ? "pill-warn"
      : state === "completed" ? "pill-ok"
      : state === "cancelled" ? ""
      : "pill-accent";
    return '<span class="pill ' + cls + '">' + esc(RUN_STATE_WORD[state] || state) + "</span>";
  }

  function runCanRetry(state) {
    return ["failed", "unverified", "paused", "blocked", "review", "completed", "cancelled"].indexOf(state) !== -1;
  }

  function runCanCancel(state) {
    return ["intake", "planning", "awaiting_approval", "executing", "verifying", "review"].indexOf(state) !== -1;
  }

  function openRun(id) {
    RUN.id = id;
    RUN.data = null;
    RUN.events = [];
    RUN.hasOlder = false;
    RUN.error = null;
    RUN.changes = null;
    uiSheet("Run", '<div class="loading">Loading the run...</div>');
    loadRun(id).then(function () {
      if (RUN.id === id) paintRunSheet();
      // The diff and the environment are read after the sheet is up: they come
      // from git and the project, and the run's own record must not wait for them.
      request("/api/runs/" + encodeURIComponent(id) + "/changes").then(function (res) {
        if (RUN.id !== id) return;
        RUN.changes = res.ok ? res.body : { error: errorOf(res.body, "the project's changes could not be read") };
        paintRunSheet();
      });
    });
  }

  function runChangesHtml() {
    var c = RUN.changes;
    if (!c) return '<div class="panel"><div class="panel-head"><span>Changes in the project</span></div><div class="panel-body"><div class="loading" role="status">Reading the project...</div></div></div>';
    if (c.error) return '<div class="panel"><div class="panel-head"><span>Changes in the project</span></div><div class="panel-body"><div class="notice notice-warn">' + esc(c.error) + "</div></div></div>";
    if (!c.project) return '<div class="panel"><div class="panel-head"><span>Changes in the project</span></div><div class="panel-body"><div class="empty-inline">' + esc(c.note || "This run is not tied to a project.") + "</div></div></div>";
    var diff = c.diff && c.diff.patch
      ? renderPatch(c.diff.patch) + (c.diff.truncated ? '<div class="dim">The diff is longer than this sheet shows (' + c.diff.bytes + " bytes). The rest is in the repository.</div>" : "")
      : c.diff_error ? '<div class="empty-inline">' + esc(c.diff_error) + "</div>"
      : '<div class="empty-inline">The working tree has no uncommitted changes.</div>';
    var observed = (c.observed || []).map(function (o) {
      var files = "";
      try { files = (JSON.parse(o.detail || "{}").files || []).slice(0, 40).map(function (f) { return "<li class=\"mono\">" + esc(f) + "</li>"; }).join(""); } catch (e) { files = ""; }
      return '<div class="run-observed"><div>' + esc(o.summary) + "</div>" + (files ? '<ul class="run-acceptance">' + files + "</ul>" : "") +
        '<div class="faint">Seen by comparing the project before and after. This does not say who made the change.</div></div>';
    }).join("");
    var env = c.environment, envHtml = "";
    if (env) {
      var list = function (items, f) { return (items || []).length ? (items || []).map(f).join(", ") : "none"; };
      envHtml = '<div class="panel"><div class="panel-head"><span>Environment</span><span class="panel-note mono">' + esc(String(env.fingerprint || "").slice(0, 12)) + "</span></div>" +
        '<div class="panel-body stack">' +
        "<div><b>Runtimes</b> " + esc(list(env.runtimes, function (r) { return (r.kind || r.name || "") + (r.version ? " " + r.version : ""); })) + "</div>" +
        "<div><b>Services</b> " + esc(list(env.services, function (s) { return (s.name || "") + (s.running === false ? " (stopped)" : ""); })) + "</div>" +
        "<div><b>Sites</b> " + esc(list(env.sites, function (s) { return s.host || s.name || ""; })) + "</div>" +
        ((env.missing || []).length ? '<div class="notice notice-warn"><b>Missing</b> ' + esc((env.missing || []).join("; ")) + "</div>" : "") +
        "</div></div>";
    } else if (c.environment_error) {
      envHtml = '<div class="panel"><div class="panel-head"><span>Environment</span></div><div class="panel-body"><div class="empty-inline">' + esc(c.environment_error) + "</div></div></div>";
    }
    return '<div class="panel"><div class="panel-head"><span>Changes in the project</span><span class="panel-note mono">' + esc(c.project) + "</span></div>" +
      '<div class="panel-body">' + observed + diff + "</div></div>" + envHtml;
  }

  function loadRun(id) {
    return request("/api/runs/" + encodeURIComponent(id)).then(function (res) {
      if (!res.ok) throw new Error(errorOf(res.body, "that run could not be read"));
      RUN.data = res.body;
      return request("/api/runs/" + encodeURIComponent(id) + "/events?limit=200").then(function (page) {
        if (page.ok) {
          RUN.events = (page.body && page.body.items) || [];
          RUN.hasOlder = RUN.events.length >= 200;
        }
      });
    }).catch(function (e) {
      RUN.error = e.message;
      RUN.data = null;
    });
  }

  function paintRunSheet() {
    if (!RUN.id) return;
    uiSheet("Run", runDetailHtml());
  }

  function runEventRow(e) {
    var at = String(e.created_at || "");
    var when = at.length > 10 ? at.slice(11, 19) : at;
    var detail = e.data && Object.keys(e.data).length ? '<details class="run-event-data"><summary>details</summary><pre>' + esc(clip(JSON.stringify(e.data, null, 2), 4000)) + "</pre></details>" : "";
    return '<div class="run-event"><span class="run-event-at mono">' + esc(when) + '</span><span class="run-event-kind">' + esc(e.kind) + '</span><span class="run-event-msg">' + esc(e.message || "") + "</span>" + detail + "</div>";
  }

  function runDetailHtml() {
    var d = RUN.data;
    if (!d) return '<div class="empty-inline">' + (RUN.error ? esc(RUN.error) : "That run could not be read.") + "</div>";
    var run = d.run || {};
    var state = run.state || "intake";
    var pending = (d.approvals || []).filter(function (a) { return a.state === "pending"; });
    var evidence = d.evidence || [];
    var passed = evidence.filter(function (e) { return e.passed; }).length;
    var failed = evidence.length - passed;
    var acceptance = (run.acceptance || []).map(function (a) { return "<li>" + esc(a) + "</li>"; }).join("");
    // A team run is started, paused and resumed from the team run itself, not
    // from its record. Offering the conversation controls here would promise a
    // behaviour the server refuses for team runs.
    var team = run.mode === "team";
    var actions =
      (run.thread_id ? '<button type="button" class="btn btn-mini" data-action="run-chat" data-id="' + attr(run.thread_id) + '">Conversation</button>' : "") +
      (team && run.team_run_id ? '<button type="button" class="btn btn-mini" data-action="run-team-open" data-id="' + attr(run.team_run_id) + '">Open the team run</button>' : "") +
      (!team && runCanCancel(state) ? '<button type="button" class="btn btn-mini" data-action="run-cancel" data-id="' + attr(run.id) + '">Stop</button>' : "") +
      (!team && runCanRetry(state) ? '<button type="button" class="btn btn-mini btn-primary" data-action="run-retry" data-id="' + attr(run.id) + '">Continue</button>' : "");
    var steps = (d.steps || []).map(function (s) {
      return '<div class="run-step"><span class="dot ' + (s.state === "done" ? "dot-ok" : s.state === "failed" ? "dot-bad" : s.state === "running" ? "dot-warn" : "dot-off") + '"></span><span class="grow"><b>' + esc(s.title) + "</b> <span class=\"dim\">" + esc(s.kind) + "</span></span><span class=\"dim\">" + esc(s.state) + "</span></div>";
    }).join("") || '<div class="empty-inline">No steps yet.</div>';
    var calls = (d.tool_calls || []).map(function (c) {
      return uiRow({
        dot: c.ok ? "ok" : "bad",
        title: esc(c.capability) + (c.exit_code !== null && c.exit_code !== undefined ? ' <span class="dim">exit ' + c.exit_code + "</span>" : ""),
        sub: esc((c.ok ? "succeeded" : c.error_code || "failed") + ", " + c.duration_ms + " ms, " + (c.surface || "") + (c.error ? ", " + c.error : "")),
        wrap: true
      });
    }).join("") || '<div class="empty-inline">No capability calls recorded.</div>';
    var ev = evidence.map(function (e) {
      return uiRow({ dot: e.passed ? "ok" : "warn", title: esc(e.summary), sub: esc(e.kind + ", " + e.capability + (e.claim ? ", supports " + e.claim : "")), wrap: true });
    }).join("") || '<div class="empty-inline">No evidence recorded. The run cannot be verified on a claim alone.</div>';
    var arts = (d.artifacts || []).map(function (a) {
      return uiRow({ title: esc(a.name), sub: esc(a.kind + ", " + a.bytes + " bytes" + (a.summary ? ", " + a.summary : "")), meta: a.path ? '<span class="dim mono">' + esc(a.path) + "</span>" : "" });
    }).join("");
    return '<div class="run-detail">' +
      '<div class="chat-head"><div class="grow"><div class="chat-title">' + esc(run.goal || "Untitled run") + '</div><div class="dim mono">' + esc(run.mode || "") + (run.project ? ", " + esc(run.project) : "") + ", started " + esc(ago(run.created_at) || "") + '</div></div>' + runPill(state) + actions + "</div>" +
      (run.error ? '<div class="notice notice-warn">' + esc(run.error) + "</div>" : "") +
      (pending.length ? '<div class="panel"><div class="panel-head"><span>Waiting for a person</span><span class="panel-note">' + pending.length + '</span></div><div class="panel-body">' + pending.map(function (a) {
        return '<div class="run-approval"><div class="grow"><div><b>' + esc(a.capability) + "</b> " + (a.risk === "destructive" ? '<span class="pill pill-bad">destructive</span>' : a.risk === "sensitive" ? '<span class="pill pill-warn">sensitive</span>' : "") + "</div>" +
          '<div class="dim">asked by ' + esc(a.actor || "") + ", expires " + esc(ago(a.expires_at) || a.expires_at || "") + "</div>" +
          '<details class="run-event-data"><summary>the exact operation</summary><pre>' + esc(clip(JSON.stringify(a.args || {}, null, 2), 4000)) + "</pre></details></div></div>";
      }).join("") + '<div class="inline" style="margin-top:8px"><button type="button" class="btn btn-mini" data-action="nav" data-view="inbox">Decide in Inbox</button></div></div></div>' : "") +
      (acceptance ? '<div class="panel"><div class="panel-head"><span>Acceptance criteria</span></div><div class="panel-body"><ul class="run-acceptance">' + acceptance + "</ul></div></div>" : "") +
      '<div class="summary-band">' +
      '<div class="summary-item"><div class="summary-label">Evidence</div><div class="summary-value">' + passed + " passed</div><div class=\"summary-detail\">" + (failed ? failed + " failed or inconclusive" : evidence.length ? "all recorded evidence supports the claim" : "none recorded") + "</div></div>" +
      '<div class="summary-item"><div class="summary-label">Steps</div><div class="summary-value">' + (d.steps || []).length + '</div><div class="summary-detail">' + (d.events || []).length + " recorded events</div></div>" +
      '<div class="summary-item"><div class="summary-label">Capability calls</div><div class="summary-value">' + (d.tool_calls || []).length + '</div><div class="summary-detail">' + (d.artifacts || []).length + " artifacts</div></div>" +
      "</div>" +
      runChangesHtml() +
      '<div class="panel"><div class="panel-head"><span>Timeline</span><span class="panel-note">newest 200</span></div><div class="panel-body flush">' +
      (RUN.hasOlder ? '<div class="inline" style="padding:8px 12px"><button type="button" class="btn btn-mini" data-action="run-older" data-id="' + attr(run.id) + '">Show earlier events</button></div>' : "") +
      (RUN.events.length ? RUN.events.map(runEventRow).join("") : '<div class="empty-inline">No events yet.</div>') + "</div></div>" +
      '<div class="panel"><div class="panel-head"><span>Steps</span></div><div class="panel-body">' + steps + "</div></div>" +
      '<div class="panel"><div class="panel-head"><span>Capability calls</span></div><div class="panel-body flush">' + calls + "</div></div>" +
      '<div class="panel"><div class="panel-head"><span>Evidence</span><span class="panel-note">' + evidence.length + "</span></div><div class=\"panel-body flush\">" + ev + "</div></div>" +
      (arts ? '<div class="panel"><div class="panel-head"><span>Artifacts</span></div><div class="panel-body flush">' + arts + "</div></div>" : "") +
      "</div>";
  }

  // ------------------------------------------------------------------ health

  var HEALTH_WORD = { healthy: "Healthy", degraded: "Degraded", starting: "Starting", unavailable: "Unavailable", critical: "Critical", setup_incomplete: "Setup incomplete", unknown: "Unknown" };

  function loadHealth() {
    if (AI.healthPending) return AI.healthPending;
    AI.healthLoading = true;
    var timeout = new Promise(function (_, rej) { setTimeout(function () { rej(new Error("health check timed out after 15 s")); }, 15000); });
    AI.healthPending = Promise.race([read("health.status"), timeout]).then(function (h) {
      AI.health = h; AI.healthError = null; AI.healthAt = Date.now();
      read("environment.get").then(function (e) { AI.env = e; }).catch(function () {});
      return Promise.all([read("approval.list", { pending_only: true }), read("inbox.list", { open_only: true })]).then(function (r) {
        AI.counts.approvals = (r[0] || []).length;
        AI.counts.questions = (r[1] || []).filter(function (m) { return m.needs_reply; }).length;
      });
    }).catch(function (e) { AI.healthError = e.message; }).then(function () {
      AI.healthLoading = false;
      AI.healthPending = null;
    });
    return AI.healthPending;
  }

  function healthStale() { return AI.health && Date.now() - AI.healthAt > 90000; }

  function healthChip() {
    var h = AI.health, chip = el("health-chip");
    if (!chip) return;
    var st = AI.healthError && !h ? "unknown" : h ? h.state : "checking";
    if (healthStale() || (AI.healthError && h) || OFFLINE) st = "stale";
    var title, sub;
    if (st === "checking") { title = "Checking…"; sub = ""; }
    else if (st === "stale" && OFFLINE) { title = "Not answering"; sub = "showing the last known state"; }
    else if (st === "stale") { title = "Status is old"; sub = AI.healthError ? "last check failed" : "checked " + Math.round((Date.now() - AI.healthAt) / 60000) + " min ago"; }
    else if (!h) { title = "Cannot check"; sub = AI.healthError; }
    else { title = HEALTH_WORD[h.state] || h.state; sub = h.state === "healthy" ? (h.notes ? h.notes + " optional note" + (h.notes === 1 ? "" : "s") : "") : h.headline; }
    chip.className = "health health-" + st;
    chip.innerHTML = '<span class="health-dot" aria-hidden="true"></span><span class="health-text"><span class="health-title">' + esc(title) + '</span>' + (sub ? '<span class="health-sub">' + esc(sub) + "</span>" : "") + "</span>";
    chip.setAttribute("aria-label", "devd status: " + title + (sub ? ", " + sub : ""));
  }

  function healthRow(c) {
    var bad = c.state !== "healthy", fix = c.fix || {};
    var btn = !bad ? "" : fix.capability
      ? '<button type="button" class="btn btn-mini btn-primary" data-action="health-fix" data-cap="' + attr(fix.capability) + '" data-args="' + attr(JSON.stringify(fix.args || {})) + '">' + esc(fix.label) + "</button>"
      : fix.view ? '<button type="button" class="btn btn-mini" data-action="health-go" data-view="' + attr(fix.view) + '">' + esc(fix.label) + "</button>" : "";
    return '<div class="hrow hstate-' + c.state + '"><span class="health-dot"></span><div class="grow"><div class="hrow-title">' + esc(c.label) + ' <span class="pill">' + esc(HEALTH_WORD[c.state] || c.state) + "</span>" + (c.required ? "" : ' <span class="dim">optional</span>') +
      '</div><div class="row-sub">' + esc(c.detail) + "</div>" + (bad ? '<div class="row-sub">' + esc(c.meaning) + "</div>" : "") + "</div>" + btn + "</div>";
  }

  function healthBody(withClose) {
    var h = AI.health;
    if (!h) return '<div class="notice notice-warn">' + esc(AI.healthError || "Not checked yet") + '</div><div class="inline"><button type="button" class="btn" data-action="health-recheck">Try again</button></div>';
    var order = { critical: 0, unavailable: 1, degraded: 2, starting: 3, unknown: 4, setup_incomplete: 5, healthy: 6 };
    var sorted = h.checks.slice().sort(function (a, b) { return order[a.state] - order[b.state]; });
    // Three different answers, kept apart: a check that failed, a check devd could not
    // run or finish, and a check that passed. Unknown is not the same as fine.
    var fails = sorted.filter(function (c) { return ["critical", "unavailable", "degraded"].indexOf(c.state) >= 0; });
    var unsure = sorted.filter(function (c) { return ["starting", "unknown", "setup_incomplete"].indexOf(c.state) >= 0; });
    var passing = sorted.filter(function (c) { return c.state === "healthy"; });
    var rows = (fails.length ? '<div class="section-title">Needs a look <span class="count">' + fails.length + "</span></div>" + fails.map(healthRow).join("") : "") +
      (unsure.length ? '<div class="section-title">Not confirmed <span class="count">' + unsure.length + '</span></div><div class="row-sub" style="margin-bottom:6px">devd could not finish these checks. That is not the same as passing.</div>' + unsure.map(healthRow).join("") : "") +
      (passing.length ? '<details class="ui-fold"><summary>All good <span class="count">' + passing.length + "</span></summary>" + passing.map(healthRow).join("") + "</details>" : "");
    var tone = h.state === "healthy" ? "notice-ok" : h.state === "critical" || h.state === "degraded" ? "notice-bad" : "notice-warn";
    return '<div class="notice ' + tone + '"><b>' + esc(h.headline) + ".</b> Checked " + esc(ago(h.checked_at) || "just now") + " in " + h.took_ms + " ms." + (healthStale() ? " This is out of date." : "") + (AI.healthError ? " The last re-check failed: " + esc(AI.healthError) : "") + "</div>" +
      (h.attention.length ? '<div class="section-title">Waiting for you</div>' + h.attention.map(function (a) { return '<div class="hrow"><span class="health-dot" style="background:var(--warn)"></span><div class="grow">' + esc(a.label) + '</div><button type="button" class="btn btn-mini" data-action="health-go" data-view="' + attr(a.view) + '">Open</button></div>'; }).join("") : "") +
      rows +
      '<div class="inline" style="margin-top:14px"><button type="button" class="btn" data-action="health-recheck"' + (AI.healthLoading ? " disabled" : "") + ">" + (AI.healthLoading ? "Checking…" : "Check again") + "</button>" + (withClose ? '<button type="button" class="btn btn-ghost" data-action="health-go" data-view="system">Open diagnostics</button>' : "") + "</div>";
  }

  function openHealth() { openDrawer("devd status", healthBody(true)); }

  // -------------------------------------------------------------------- home

  function loadHome() {
    // The quick reads go out first and the page draws when they arrive. The health check and
    // the machine inventory probe the whole computer and take seconds; they are asked for
    // afterwards and fill in when ready.
    var fast = Promise.all([
      read("project.list").catch(function () { return []; }),
      read("approval.list", { pending_only: true }).catch(function () { return []; }),
      read("inbox.list", { open_only: true }).catch(function () { return []; }),
      read("chat.list").catch(function () { return []; }),
      read("provider.list").catch(function () { return []; }),
      read("team.list").catch(function () { return []; }),
      request("/api/audit?limit=300").then(function (r) { return r.ok ? r.body.items || [] : []; })
    ]);
    var slowDone = fast.then(function () {
      return Promise.all([
        loadHealth(),
        read("setup.requirements").catch(function () { return null; }),
        state.server.loaded ? Promise.resolve() : loadServer()
      ]);
    }).then(function (r) {
      AI.requirements = r[1];
      if (AI.home) AI.home.requirements = r[1];
      if (state.view === "home") render();
    });
    void slowDone;
    return fast.then(function (r) {
      AI.home = {
        projects: r[0] || [],
        approvals: r[1] || [],
        inbox: r[2] || [],
        threads: r[3] || [],
        providers: r[4] || [],
        teamRuns: r[5] || [],
        audit: r[6] || [],
        requirements: AI.requirements || null
      };
      AI.providers = r[4] || [];
      state.projectCount = AI.home.projects.length;
    });
  }

  function greeting() {
    var h = new Date().getHours();
    return h < 5 ? "Still up" : h < 12 ? "Good morning" : h < 18 ? "Good afternoon" : "Good evening";
  }

  function approvalCard(r, compact) {
    var steps = (r.plan && r.plan.steps || []).slice(0, compact ? 3 : 12).map(function (x) { return "<li>" + esc(x) + "</li>"; }).join("");
    var effects = (r.plan && r.plan.effects || []).slice(0, 4).map(function (x) { return '<li class="warn-text">' + esc(x) + "</li>"; }).join("");
    return '<div class="ask ask-approval"><div class="ask-head"><span class="pill pill-warn">approval</span><b>' + esc(r.capability) + '</b><span class="dim">' + esc(r.actor.replace("agent:", "")) + " · " + esc(ago(r.created_at)) + "</span></div>" +
      ((r.plan && r.plan.summary) ? '<div class="ask-body">' + esc(r.plan.summary) + "</div>" : "") +
      (steps || effects ? '<ul class="ask-steps">' + steps + effects + "</ul>" : "") +
      '<div class="mono ask-args nowrap" title="' + attr(JSON.stringify(r.args)) + '">' + esc(JSON.stringify(r.args)) + "</div>" +
      '<div class="inline"><button type="button" class="btn btn-go btn-mini" data-action="req-decide" data-id="' + attr(r.id) + '" data-approve="1">Approve and run</button>' +
      '<button type="button" class="btn btn-mini btn-danger" data-action="req-decide" data-id="' + attr(r.id) + '" data-approve="0">Decline</button></div></div>';
  }

  function messageCard(m) {
    return '<div class="ask"><div class="ask-head"><span class="pill ' + (m.needs_reply ? "pill-accent" : "") + '">' + (m.needs_reply ? "question" : "message") + "</span><b>" + esc(m.subject) + '</b><span class="dim">' + esc(m.from_actor.replace("agent:", "")) + " · " + esc(ago(m.created_at)) + "</span></div>" +
      '<div class="ask-body">' + md(m.body) + "</div>" +
      (m.needs_reply ? '<div class="inline"><input class="input grow" id="reply-' + attr(m.id) + '" placeholder="Your answer"><button type="button" class="btn btn-primary btn-mini" data-action="inbox-reply" data-id="' + attr(m.id) + '">Send</button></div>'
        : '<div class="inline"><button type="button" class="btn btn-mini" data-action="inbox-reply" data-id="' + attr(m.id) + '" data-resolve="1">Mark as read</button></div>') + "</div>";
  }

  /* A requirement row: what it is, why devd wants it, and the one action that
     fixes it. The action is the ordinary capability call, so it asks for
     approval like everything else that changes the machine. */
  function requirementRow(r) {
    var fix = r.fix || {};
    var action = "";
    if (fix.capability) {
      action = '<button type="button" class="btn btn-mini btn-primary" data-action="req-fix" data-id="' + attr(r.id) + '">' + esc(fix.label || "Install") + "</button>";
    } else if (fix.view) {
      action = '<button type="button" class="btn btn-mini" data-action="req-fix" data-id="' + attr(r.id) + '">' + esc(fix.label || "Open") + "</button>";
    } else if (fix.command || fix.url) {
      action = '<button type="button" class="btn btn-mini" data-action="req-how" data-id="' + attr(r.id) + '">How</button>';
    }
    var icon = r.state === "broken" ? "i-warn" : r.state === "missing" ? "i-plus" : "i-check";
    var word = r.state === "broken" ? "Not working" : r.state === "missing" ? "Not installed" : "Ready";
    return '<div class="req-row req-' + attr(r.state || "missing") + '"><span class="req-icon" aria-hidden="true"><svg class="icon"><use href="#' + icon + '"></use></svg></span>' +
      '<div class="req-main"><div class="req-title">' + esc(r.title) + ' <span class="req-state">' + word + "</span>" +
      '</div><div class="row-sub">' + esc(r.why) + "</div>" +
      (r.detail ? '<div class="row-sub mono">' + esc(r.detail) + "</div>" : "") + "</div>" +
      '<div class="req-action">' + action + "</div></div>";
  }

  function requirementPanel() {
    var reqs = (AI.requirements && AI.requirements.requirements) || [];
    var outstanding = reqs.filter(function (r) { return r.state !== "ready"; });
    if (!outstanding.length) {
      return '<div class="section-title">Ready</div><div class="calm"><span class="health-dot"></span><div><b>Everything devd needs is here.</b><div class="row-sub">Git, PHP, the services and a model provider are all in place.</div></div></div>';
    }
    return '<div class="section-title">Needs installing <span class="count">' + outstanding.length + "</span></div>" +
      '<div class="panel">' + outstanding.map(requirementRow).join("") + "</div>";
  }

  function findRequirement(id) {
    var reqs = (AI.requirements && AI.requirements.requirements) || [];
    return reqs.filter(function (r) { return r.id === id; })[0] || null;
  }

  function approvalRow(r) {
    var title = (r.plan && r.plan.summary) || ("Run " + r.capability);
    return uiRow({
      accent: true, dot: "warn",
      title: esc(title),
      sub: esc(r.actor.replace("agent:", "") + " asks to use " + r.capability),
      meta: esc(ago(r.created_at)),
      open: 'data-action="open-approval" data-id="' + attr(r.id) + '"',
      actions: '<button type="button" class="btn btn-go btn-mini" data-action="req-decide" data-id="' + attr(r.id) + '" data-approve="1">Approve</button>' +
        '<button type="button" class="btn btn-mini btn-ghost" data-action="req-decide" data-id="' + attr(r.id) + '" data-approve="0">Decline</button>'
    });
  }

  function questionRow(m) {
    var who = m.from_actor.replace("agent:", "").replace("devd-", "");
    return uiRow({
      accent: true, dot: "warn",
      title: esc(m.subject), sub: esc(uiSummary(m.body, 170)), wrap: true,
      meta: esc(who + " · " + ago(m.created_at)),
      open: 'data-action="open-message" data-id="' + attr(m.id) + '"',
      actions: '<button type="button" class="btn btn-primary btn-mini" data-action="open-message" data-id="' + attr(m.id) + '">Answer</button>'
    });
  }

  function renderHome() {
    if (state.view !== "home") return;
    var d = AI.home;
    if (!d) return renderLoading("Reading projects, approvals and the services on this machine.");
    var h = AI.health || { level: "checking", checks: [], headline: "" };
    var ready = d.providers.some(function (p) { return p.enabled && p.has_key; }) || d.providers.some(function (p) { return p.enabled && !p.needs_key; });
    var projOptions = '<option value="">No project</option>' + d.projects.map(function (p) { return '<option value="' + attr(p.path) + '"' + (AI.askProject === p.path ? " selected" : "") + ">" + esc(p.name) + "</option>"; }).join("");

    // The composer is one object: what to do, who does it, and where. The mode
    // decides who acts - the assistant for one request, the team for a goal with
    // a plan, approval and proof - and the button says which will happen.
    var mode = AI.homeMode === "team" ? "team" : "ask";
    // The composer. One surface for one act: say what you want, choose who does it
    // and where, send. The mode decides who acts - the assistant answers one
    // request; the team plans a goal and waits for approval - and the button
    // names which will happen.
    var team = mode === "team";
    var ask = ready
      ? '<div class="composer" role="group" aria-label="Tell devd what you are working on">' +
          composerTop("home") +
          '<label class="sr-only" for="home-ask">' + (team ? "The outcome you want from the team" : "What would you like to ask devd?") + "</label>" +
          '<textarea id="home-ask" class="composer-input" rows="2" placeholder="' + (team ? "Describe the outcome you want. The team will draft a plan and wait for your go-ahead." : "What are we working on?") + '">' + esc(AI.askText || "") + "</textarea>" +
          '<div class="composer-bar">' +
            '<div class="composer-left">' +
              modeSelect() +
              '<label class="composer-where"><svg class="icon" aria-hidden="true"><use href="#i-folder"></use></svg><span class="sr-only">Project</span>' +
                '<select id="home-project">' + projOptions + "</select></label>" +
              attachButton("home") +
            "</div>" +
            '<div class="composer-right"><button type="button" class="composer-send" id="home-send" data-action="' + (team ? "home-team" : "home-ask") + '"' + (String(AI.askText || "").trim() ? "" : " disabled") + ' aria-label="' + (team ? "Ask the team to plan this" : "Ask devd") + '" title="' + (team ? "Ask the team to plan this" : "Ask devd") + '"><svg class="icon" aria-hidden="true"><use href="#i-arrow-up"></use></svg></button></div>' +
          "</div>" +
        "</div>"
      : '<div class="composer composer-off"><div class="composer-offline"><b>No model is connected.</b> Connect one to give devd goals. Everything else on this page works without it.</div>' +
          '<button type="button" class="btn btn-primary" data-action="nav" data-view="providers">Connect a model</button></div>';

    var needs = [];
    (d.teamRuns || []).filter(function (t) { return t.state === "awaiting_plan"; }).forEach(function (t) {
      needs.push(uiRow({ accent: true, dot: "warn", title: esc(t.title), sub: "The team has a plan with " + t.tasks_total + " task(s). Nothing runs until you approve it.", meta: esc(ago(t.updated_at)),
        actions: '<button type="button" class="btn btn-primary btn-mini" data-action="team-open-from" data-id="' + attr(t.id) + '">Review plan</button>' }));
    });
    (d.teamRuns || []).filter(function (t) { return t.state === "paused" && t.error; }).forEach(function (t) {
      needs.push(uiRow({ accent: true, dot: "warn", title: esc(t.title), sub: esc(uiSummary(t.error, 150)), wrap: true, meta: "paused",
        actions: '<button type="button" class="btn btn-mini" data-action="team-open-from" data-id="' + attr(t.id) + '">Open</button>' }));
    });
    d.approvals.forEach(function (r) { needs.push(approvalRow(r)); });
    d.inbox.filter(function (m) { return m.needs_reply; }).forEach(function (m) { needs.push(questionRow(m)); });
    (h.checks || []).filter(function (c) { return c.required && c.state !== "healthy" && c.state !== "starting"; }).forEach(function (c) {
      var f = c.fix || {}, bad = c.state === "critical" || c.state === "unavailable" || c.state === "degraded";
      var act = f.capability
        ? '<button type="button" class="btn btn-primary btn-mini" data-action="health-fix" data-cap="' + attr(f.capability) + '" data-args="' + attr(JSON.stringify(f.args || {})) + '">' + esc(f.label) + "</button>"
        : f.view ? '<button type="button" class="btn btn-mini" data-action="health-go" data-view="' + attr(f.view) + '">' + esc(f.label) + "</button>" : "";
      needs.push(uiRow({ accent: true, dot: bad ? "bad" : "warn", title: esc(c.label), sub: esc(c.meaning || c.detail), wrap: true, actions: act }));
    });
    var updates = d.inbox.filter(function (m) { return !m.needs_reply; }).length;
    var needsHtml = "";
    if (needs.length) {
      needsHtml += uiHead("Needs you", needs.length) + uiList(needs.slice(0, 4));
      var hidden = needs.length - 4 + updates;
      if (hidden > 0) needsHtml += '<div class="ui-more"><button type="button" data-action="nav" data-view="inbox">' + hidden + " more in the inbox</button></div>";
    } else if (updates) {
      needsHtml += '<div class="ui-more"><button type="button" data-action="nav" data-view="inbox">' + updates + " update" + (updates === 1 ? "" : "s") + " in the inbox</button></div>";
    }

    var reqs = ((AI.requirements && AI.requirements.requirements) || []).filter(function (r) { return r.state !== "ready"; });
    // Only what devd needs is "setup". Something optional is offered, not nagged about.
    var required = reqs.filter(function (r) { return r.required; });
    var setup = reqs.length
      ? uiList([uiRow({ dot: required.length ? "warn" : "off", title: required.length ? "Let's finish setting up" : "When you're ready", sub: (required.length ? required.length + " needed" + (reqs.length > required.length ? ", " + (reqs.length - required.length) + " optional" : "") : "Whenever you like") + ": " + esc(reqs.slice(0, 3).map(function (r) { return r.title; }).join(", ")) + (reqs.length > 3 ? "…" : ""),
          actions: '<button type="button" class="btn btn-mini" data-action="open-setup">Review</button>' })])
      : "";

    // Projects and sites are two views of the same work. The map is built here,
    // once, so a project row can say which address serves it without a second
    // capability call or a guess about the folder.
    var siteFor = {};
    (state.server.sites || []).forEach(function (row) {
      if (row.project && !siteFor[row.project]) siteFor[row.project] = row;
    });
    var allSites = state.server.sites || [];
    var allProjects = d.projects || [];
    var exposed = allProjects.filter(function (p) { return siteFor[p.path]; }).length;

    var projects = allProjects.length
      ? uiList(allProjects.slice(0, 8).map(function (p) {
        var site = siteFor[p.path];
        return uiRow({
          dot: site ? (site.serving === "no_entry" ? "warn" : "ok") : "off",
          title: '<span class="row-name">' + esc(p.name) + "</span>" + ((p.stack || site) ? ' <span class="facts">' +
            (p.stack ? '<span class="fact">' + esc(p.stack) + "</span>" : "") +
            (site ? '<span class="fact mono">' + esc(site.site.host || site.site.name) + "</span>" : "") + "</span>" : ""),
          sub: '<span class="mono">' + esc(p.path) + "</span>",
          meta: site ? esc(servingLabel(site)) : "no address",
          open: 'data-action="home-open" data-path="' + attr(p.path) + '"',
          actions: '<button type="button" class="btn btn-mini btn-primary" data-action="home-open" data-path="' + attr(p.path) + '">Open</button>' +
            (site ? '<button type="button" class="btn btn-mini" data-action="open-site" data-name="' + attr(site.site.name) + '">Site</button>' : "") +
            (ready ? '<button type="button" class="btn btn-mini btn-ghost" data-action="home-chat" data-path="' + attr(p.path) + '">Ask</button>' : ""),
          hover: true
        });
      }))
      : uiEmpty("Your workspace is empty", "Start from a stack, or point devd at a folder you already have.",
        '<div class="inline"><button type="button" class="btn btn-primary" data-action="nav" data-view="create">Create a project</button><button type="button" class="btn" data-action="nav" data-view="develop">Import a folder</button></div>');

    var counts = serviceCounts(state.server.services);
    var noEntry = allSites.filter(function (row) { return row.serving === "no_entry"; });
    var siteAttention = allSites.filter(siteNeedsAttention);
    var pendingHosts = ((state.server.hosts && (state.server.hosts.adding || []).concat(state.server.hosts.removing || [])) || []).length;
    var phps = (state.server.runtimes || []).filter(function (r) { return r.runtime && r.runtime.kind === "php"; });
    var defaultPhp = phpPrimary();

    // Running is not the same as answering. A service counts as ready only when
    // it answered a question in its own protocol; one that holds its port and
    // says nothing is reported as such.
    var svcRows = (state.server.services || []).filter(function (r) { return r.installed; });
    var svcRunning = svcRows.filter(function (r) { return r.running; });
    var svcReady = svcRunning.filter(function (r) { return r.ready === true; });
    var svcSilent = svcRunning.filter(function (r) { return r.ready === false; });
    var svcHeld = (state.server.services || []).filter(function (r) { return !r.running && r.listening; });
    var working = d.threads.filter(function (t) { return t.state === "running"; }).length +
      (d.teamRuns || []).filter(function (t) { return ["planning", "running"].indexOf(t.state) >= 0; }).length;
    var waiting = d.approvals.length + (d.teamRuns || []).filter(function (t) { return t.state === "awaiting_plan"; }).length;

    function cell(view, label, dot, value, detail) {
      return '<button type="button" class="env-cell" data-action="nav" data-view="' + view + '">' +
        '<span class="env-cell-label">' + uiDot(dot) + esc(label) + "</span>" +
        '<span class="env-cell-value">' + esc(value) + "</span>" +
        '<span class="env-cell-detail">' + esc(detail) + "</span></button>";
    }
    var env = state.server.loaded
      ? '<div class="env-strip" role="group" aria-label="This machine">' +
        cell("services", "Services", !counts.total ? "off" : svcSilent.length || svcHeld.length ? "warn" : counts.up === counts.total ? "ok" : counts.up ? "warn" : "off",
          counts.total ? counts.up + " of " + counts.total + " running" : "None installed",
          svcSilent.length ? svcSilent.length + " running but not answering" :
          svcHeld.length ? svcHeld.length + (svcHeld.length === 1 ? " port" : " ports") + " held by other programs" : svcRunning.length ? svcReady.length + " answered in their own protocol" : counts.total ? "start them when you need them" : "install a web server and a database") +
        cell("sites", "Sites", noEntry.length || siteAttention.length ? "warn" : allSites.length ? "ok" : "off",
          allSites.length ? (allSites.length - noEntry.length) + " of " + allSites.length + " can serve" : "No sites",
          noEntry.length ? noEntry.length + " with nothing to serve" : pendingHosts ? pendingHosts + " hostname" + (pendingHosts === 1 ? "" : "s") + " not published" : allSites.length ? "HTTPS from devd's own authority" : "give a project an address") +
        cell("runtimes", "Runtimes", (state.server.runtimes || []).length ? "ok" : "off",
          (state.server.runtimes || []).length ? (state.server.runtimes || []).length + " installed" : "None installed",
          phps.length ? "PHP default " + (defaultPhp || "not selected") : "PHP, Node.js, Go, Python, Ruby") +
        cell("team", "Agents", waiting ? "warn" : working ? "live" : "off",
          working ? working + " working" : waiting ? waiting + " waiting for you" : "Idle",
          waiting && working ? waiting + " waiting for you" : ready ? "plans need your approval before they run" : "connect a model to use agents") +
        "</div>"
      : '<div class="loading" role="status">Reading the environment…</div>';
    var envLinks = (counts.up < counts.total || noEntry.length || pendingHosts)
      ? '<div class="env-links">' +
        (counts.up < counts.total ? '<button type="button" class="btn btn-mini" data-action="stack-up">Start all services</button>' : "") +
        (noEntry.length ? '<button type="button" class="btn btn-mini" data-action="nav" data-view="sites">Review ' + noEntry.length + " site" + (noEntry.length === 1 ? "" : "s") + "</button>" : "") +
        (pendingHosts ? '<button type="button" class="btn btn-mini" data-action="nav" data-view="hosts">Publish ' + pendingHosts + " hostname" + (pendingHosts === 1 ? "" : "s") + "</button>" : "") +
        "</div>"
      : "";

    var sites = allSites.length
      ? uiList(allSites.slice(0, 5).map(function (row) {
        return uiRow({
          dot: row.serving === "no_entry" ? "warn" : row.server_running ? "ok" : "off",
          title: '<span class="mono">' + esc(row.site.host || row.site.name) + "</span>",
          sub: esc(servingLabel(row) || "not serving"),
          open: 'data-action="open-site" data-name="' + attr(row.site.name) + '"',
          actions: '<button type="button" class="btn btn-mini" data-action="open-site" data-name="' + attr(row.site.name) + '">Manage</button>' +
            '<button type="button" class="btn btn-mini btn-ghost" data-action="open-url" data-url="' + attr(row.url) + '" aria-label="Open ' + attr(row.url) + ' in the browser">Open</button>',
          hover: true
        });
      }))
      : uiEmpty("No sites yet", "A site gives a project an HTTPS address on this machine.", '<button type="button" class="btn btn-primary btn-mini" data-action="new-site">Create a site</button>');

    var live = [];
    d.threads.filter(function (t) { return t.state === "running" || t.state === "waiting_approval"; }).forEach(function (t) {
      live.push(uiRow({ dot: t.state === "running" ? "live" : "warn", title: esc(t.title), sub: t.state === "running" ? "Assistant · working" : "Assistant · waiting for your decision",
        actions: '<button type="button" class="btn btn-mini" data-action="open-thread" data-id="' + attr(t.id) + '">Open</button>' }));
    });
    (d.teamRuns || []).filter(function (t) { return ["planning", "running"].indexOf(t.state) >= 0; }).forEach(function (t) {
      live.push(uiRow({ dot: "live", title: esc(t.title), sub: "Team · " + esc(t.state === "planning" ? "planning" : (t.tasks_done || 0) + " of " + (t.tasks_total || 0) + " tasks done"),
        actions: '<button type="button" class="btn btn-mini" data-action="team-open-from" data-id="' + attr(t.id) + '">Open</button>' }));
    });

    // What happened, newest first, as a timeline. A refusal is shown as one.
    var events = d.audit.filter(function (a) { return !a.dry_run && a.tier !== "read"; }).slice(0, 8);
    var feed = events.length
      ? '<div class="timeline">' + events.map(function (a) {
        var who = a.actor.replace("human:", "you via ").replace("agent:", "agent ");
        return '<div class="tl-item">' + uiDot(a.ok ? (a.actor.indexOf("agent:") === 0 ? "live" : "ok") : "bad") +
          '<span class="tl-text" title="' + attr(a.capability + " · " + (a.message || "")) + '">' + esc(who) + (a.ok ? " ran " : " was refused ") + '<span class="mono">' + esc(a.capability) + "</span></span>" +
          '<span class="tl-when">' + esc(ago(a.at)) + "</span></div>";
      }).join("") + "</div>" +
        '<div class="ui-more"><button type="button" data-action="nav" data-view="activity">All activity</button></div>'
      : '<div class="ui-quiet">Quiet lately. Whenever something changes, from a window, an agent or a script, it lands here and in Activity.</div>';

    el("main").innerHTML =
      '<div class="page home">' +
      '<header class="home-head">' +
        '<div class="home-head-text">' +
          '<h1 class="home-status" id="home-status">' + esc(greeting()) + ".</h1>" +
          '<p class="home-lead">' + esc(homeLead(needs.length, d, required.length)) + "</p>" +
          '<p class="home-sub">' + esc(homeSentence(d, counts, allSites, noEntry, svcHeld)) + "</p>" +
        "</div>" +
      "</header>" +
      ask +
      '<div class="home-grid">' +
        '<div class="home-col home-main-col">' +
          (needsHtml ? '<section class="home-attention" aria-label="Needs you">' + needsHtml + "</section>" : "") + setup +
          (live.length ? uiHead("Working now", live.length) + uiList(live) : "") +
          uiHead("Projects", allProjects.length || null, '<button type="button" class="btn btn-mini btn-ghost" data-action="nav" data-view="develop">Import one</button><button type="button" class="btn btn-mini" data-action="nav" data-view="create">Start a new one</button><button type="button" class="btn btn-mini btn-ghost" data-action="nav" data-view="projects">See all</button>') + projects +
        "</div>" +
        '<aside class="home-col home-side-col" aria-label="This machine and recent activity">' +
          uiHead("This machine", null, '<button type="button" class="btn btn-mini btn-ghost" data-action="nav" data-view="overview">Environment</button>') +
          '<section class="home-env" aria-label="This machine">' + env + envLinks + "</section>" +
          uiHead("Recently", null) + feed +
        "</aside>" +
      "</div>" +
      "</div>";
  }

  /* What to say first, in the voice of someone glad you are here. Every branch is
     a real state; none is chosen at random, so the same situation always reads the
     same way. */
  function homeLead(needCount, d, setupCount) {
    var working = (d.teamRuns || []).filter(function (t) { return ["planning", "running"].indexOf(t.state) >= 0; });
    if (needCount) return needCount === 1 ? "One thing is waiting for your say-so." : needCount + " things are waiting for your say-so.";
    if (setupCount) return "Let's finish setting devd up, then you're free to build.";
    if (working.length) return working.length === 1 ? "The team is on it: " + String(working[0].title || "a goal") + "." : working.length + " goals are in motion. The team has it from here.";
    var h = new Date().getHours();
    if (h >= 22 || h < 5) return "All quiet. A good hour to get something done, or to call it a night.";
    return "All quiet. Pick up where you left off, or start something new.";
  }

  /* The sentence under the headline: only things devd has measured, in the order
     a person would ask about them. Nothing here is a total for its own sake. */
  function homeSentence(d, counts, allSites, noEntry, held) {
    var bits = [];
    if (counts.total) bits.push(counts.up + " of " + counts.total + " services running");
    if (held && held.length) bits.push(held.length + (held.length === 1 ? " port is" : " ports are") + " held by other programs");
    if (allSites.length) bits.push((allSites.length - noEntry.length) + " of " + allSites.length + " sites can serve");
    var working = d.threads.filter(function (t) { return t.state === "running"; }).length +
      (d.teamRuns || []).filter(function (t) { return ["planning", "running"].indexOf(t.state) >= 0; }).length;
    bits.push(working ? working + (working === 1 ? " agent is working" : " agents are working") : "no agents working");
    return bits.join("  ·  ");
  }

  /* Home's headline says what state things are in, in words: what needs a person
     first, then what is working, then quiet. It is never a question to the user. */
  function homeHeadline(needCount, d, setupCount) {
    if (needCount) return needCount === 1 ? "One thing needs you" : needCount + " things need you";
    if (setupCount) return "devd is almost set up";
    var working = d.threads.filter(function (t) { return t.state === "running"; }).length +
      (d.teamRuns || []).filter(function (t) { return ["planning", "running"].indexOf(t.state) >= 0; }).length;
    if (working) return working === 1 ? "An agent is working" : working + " agents are working";
    var counts = serviceCounts(state.server.services || []);
    if (counts.total && counts.up === counts.total) return "Everything is running";
    return "Nothing needs you";
  }

  function openSetupSheet() {
    var reqs = ((AI.requirements && AI.requirements.requirements) || []).filter(function (r) { return r.state !== "ready"; });
    uiSheet("Getting devd ready", reqs.length ? '<p class="sheet-lead">' + (reqs.filter(function (r) { return r.required; }).length ? reqs.filter(function (r) { return r.required; }).length + " of " + reqs.length + " still needed. The rest can wait." : "Nothing here is needed to work. " + (reqs.length === 1 ? "One optional step is" : reqs.length + " optional steps are") + " left.") + "</p>" + [[true, "Needed"], [false, "Whenever you like"]].map(function (g) { var rows = reqs.filter(function (r) { return !!r.required === g[0]; }); return rows.length ? uiHead(g[1], rows.length) + '<div class="panel">' + rows.map(requirementRow).join("") + "</div>" : ""; }).join("") : uiEmpty("Everything devd needs is here"));
  }

  // ------------------------------------------------------------- providers

  function loadProviders() {
    return Promise.all([read("provider.list"), read("provider.presets"), read("models.catalogue"), read("models.roles"), read("team.roles")]).then(function (r) {
      AI.providers = r[0]; AI.presets = r[1]; AI.catalogue = r[2]; AI.roleAssign = r[3]; AI.teamRoles = r[4];
    });
  }

  function money(v) { return v === null || v === undefined ? "–" : "$" + (v < 1 ? v.toFixed(2) : String(v)); }
  function tok(v) { return !v ? "–" : v >= 1000000 ? (v / 1000000) + "M" : Math.round(v / 1000) + "K"; }

  function curlExample(kind, base, env) {
    if (kind === "anthropic") return 'curl ' + base + '/v1/messages \\\n  -H "x-api-key: $' + (env || "ANTHROPIC_API_KEY") + '" \\\n  -H "anthropic-version: 2023-06-01" \\\n  -H "content-type: application/json" \\\n  -d \'{"model":"MODEL","max_tokens":256,"messages":[{"role":"user","content":"Hello"}]}\'';
    return 'curl ' + base + '/chat/completions \\\n  -H "Authorization: Bearer $' + (env || "API_KEY") + '" \\\n  -H "content-type: application/json" \\\n  -d \'{"model":"MODEL","messages":[{"role":"user","content":"Hello"}]}\'';
  }

  function modelOptions(providerId) {
    var seen = {}, out = [];
    (AI.catalogue || []).filter(function (m) { return m.source === "provider:" + providerId || (m.source === "anthropic-docs" && providerKind(providerId) === "anthropic"); }).forEach(function (m) {
      if (!seen[m.model_id]) { seen[m.model_id] = 1; out.push(m); }
    });
    return out;
  }
  function providerKind(id) { var p = (AI.providers || []).filter(function (x) { return x.id === id; })[0]; return p ? p.kind : ""; }


  function rolesPanel() {
    var provs = (AI.providers || []).filter(function (p) { return p.enabled; });
    if (!provs.length) return "";
    var info = {};
    (AI.teamRoles || []).forEach(function (r) { info[r.id] = r; });
    var rows = ["default"].concat((AI.teamRoles || []).map(function (r) { return r.id; })).map(function (role) {
      var mine = (AI.roleAssign ? AI.roleAssign.assignments : []).filter(function (a) { return a.role === role && !a.scope; }).sort(function (a, b) { return a.position - b.position; });
      var cell = function (i) {
        var a = mine[i] || {};
        var opts = '<option value="">' + (i ? "no fallback" : "use the default provider") + "</option>" + provs.map(function (p) { return '<option value="' + attr(p.id) + '"' + (a.provider === p.id ? " selected" : "") + ">" + esc(p.label) + "</option>"; }).join("");
        return '<div class="inline"><select class="input role-provider" id="rm-' + role + "-p" + i + '" aria-label="Provider for ' + attr(role) + '">' + opts + '</select><input class="input mono role-model" list="rm-models" id="rm-' + role + "-m" + i + '" value="' + attr(a.model || "") + '" placeholder="model" spellcheck="false" aria-label="Model for ' + attr(role) + '">' +
          (a.model ? '<span class="pill ' + (a.status === "available" ? "pill-ok" : a.status === "missing" ? "pill-bad" : "") + '" title="' + (a.status === "available" ? "This provider's own list includes it" : a.status === "missing" ? "Not in the list this key can see. Fetch the provider's models to refresh it." : "Fetch the provider's models to check this account can use it") + '">' + esc(a.status) + "</span>" : "") + "</div>";
      };
      var i = info[role] || { title: "Everything else", summary: "Used when a kind of work has no model of its own, and for the assistant." };
      return '<tr><td><div class="cell-strong">' + esc(i.title || role) + '</div><div class="cell-dim">' + esc(i.summary) + "</div></td><td>" + cell(0) + "</td><td>" + cell(1) + '</td><td><button type="button" class="btn btn-mini btn-primary" data-action="role-save" data-role="' + attr(role) + '">Save</button></td></tr>';
    }).join("");
    var all = {};
    (AI.providers || []).forEach(function (p) { if (p.default_model) all[p.default_model] = 1; });
    (AI.catalogue || []).filter(function (m) { return String(m.source || "").indexOf("provider:") === 0; })
      .forEach(function (m) { all[m.model_id] = 1; });
    return '<div class="section-title">Which model does which work</div><div class="panel"><div class="table-wrap"><table class="table roles-table"><thead><tr><th>Work</th><th>First choice</th><th>Fallback if it fails</th><th></th></tr></thead><tbody>' + rows + "</tbody></table><datalist id=\"rm-models\">" + Object.keys(all).map(function (m) { return '<option value="' + attr(m) + '"></option>'; }).join("") + "</datalist></div>" +
      '<div class="panel-body panel-foot row-sub">The fallback is used only when the first choice fails, and only the providers you list here are ever used. A conversation where you pick a model yourself never falls back. Each reply records which model answered and why.</div></div>';
  }

  function providerChips(p) {
    var out = [];
    if (p.category === "local") out.push('<span class="cap cap-accent">local</span>');
    else if (p.category === "aggregator") out.push('<span class="cap">aggregator</span>');
    if (p.tools === "documented") out.push('<span class="cap cap-ok">tools documented</span>');
    else if (p.tools === "none") out.push('<span class="cap cap-warn">no tool API</span>');
    else out.push('<span class="cap">tools: model-dependent</span>');
    if (p.discovery === "catalog") out.push('<span class="cap">maintained list</span>');
    if (p.discovery === "ollama") out.push('<span class="cap">native discovery</span>');
    if (p.needs_key && !p.has_key) out.push('<span class="cap cap-warn">no key</span>');
    if (!p.needs_key) out.push('<span class="cap">no key needed</span>');
    return '<div class="caps">' + out.join("") + "</div>";
  }

  /* Which model does which work, and where that computation happens, stated from
     the saved assignments. A kind of work with no assignment uses the default; it
     is counted, not listed, so the table stays about decisions someone made. */
  function modelsInUse() {
    var provs = AI.providers || [];
    var by = {};
    provs.forEach(function (p) { by[p.id] = p; });
    var def = provs.filter(function (p) { return p.is_default && p.enabled; })[0] || provs.filter(function (p) { return p.enabled; })[0];
    var assignments = (AI.roleAssign && AI.roleAssign.assignments) || [];
    var titles = {};
    (AI.teamRoles || []).forEach(function (r) { titles[r.id] = r.title || r.id; });
    function where(p) {
      if (!p) return "";
      if (p.category === "local") return "on this machine";
      try { return "remote - " + new URL(p.base_url).host; } catch (e) { return "remote"; }
    }
    function who(provId, model) {
      var p = by[provId];
      return p ? esc(p.label) + (model || p.default_model ? ' <span class="mono">' + esc(model || p.default_model) + "</span>" : "") : '<span class="dim">provider removed</span>';
    }
    var roles = [];
    assignments.filter(function (a) { return !a.scope; }).forEach(function (a) { if (roles.indexOf(a.role) < 0) roles.push(a.role); });
    var rows = [];
    rows.push('<tr><td><b>Everything else</b><div class="mu-sub">the assistant, and any work with no model of its own</div></td><td>' + (def ? who(def.id, def.default_model) : '<span class="dim">no provider enabled</span>') + "</td><td>" + esc(where(def)) + '</td><td><span class="dim">-</span></td></tr>');
    roles.forEach(function (role) {
      var mine = assignments.filter(function (a) { return a.role === role && !a.scope; }).sort(function (a, b) { return a.position - b.position; });
      var first = mine[0], second = mine[1];
      var fp = first && by[first.provider];
      rows.push("<tr><td><b>" + esc(role === "default" ? "Default" : titles[role] || role) + "</b></td><td>" + (first ? who(first.provider, first.model) : "") + "</td><td>" + esc(where(fp)) + "</td><td>" +
        (second ? who(second.provider, second.model) + ' <span class="dim">if the first fails</span>' : '<span class="dim">none - a failure stops the task</span>') + "</td></tr>");
    });
    var unassigned = (AI.teamRoles || []).filter(function (r) { return roles.indexOf(r.id) < 0; }).length;
    return uiHead("In use", null) +
      '<table class="mu"><thead><tr><th>Work</th><th>Model</th><th>Runs</th><th>Fallback</th></tr></thead><tbody>' + rows.join("") + "</tbody></table>" +
      (unassigned ? '<div class="ui-more">' + unassigned + (unassigned === 1 ? " kind of agent work has" : " kinds of agent work have") + " no model of its own and use the default. Change this under <b>Which model does which work</b> below.</div>" : "");
  }

  function providerCard(p) {
    var busy = AI.provBusy[p.id];
    var models = modelOptions(p.id);
    var source = p.key_source === "env" ? "from " + p.key_env : p.has_key ? "stored privately" : "";
    var keyState = p.has_key
      ? '<span class="pill pill-ok" title="' + attr(source) + '">key ' + esc(p.key_hint || "") + "</span>"
      : p.needs_key ? '<span class="pill pill-warn">no key</span>' : '<span class="pill">none needed</span>';
    var opts = models.map(function (m) { return '<option value="' + attr(m.model_id) + '"></option>'; }).join("");
    var result = AI.provBusy[p.id + ":msg"];
    return '<article class="provider' + (p.is_default ? " is-default" : "") + '">' +
      "<div>" +
        '<div class="provider-name">' + brandImg(p.preset || p.id, "brand-dot brand-dot-lead") + esc(p.label) +
          (p.is_default ? ' <span class="pill pill-accent">default</span>' : "") +
          (p.enabled ? "" : ' <span class="pill">disabled</span>') +
          ' <span class="pill">' + esc(p.kind) + "</span></div>" +
        '<div class="provider-meta">' + esc(p.base_url) + "</div>" +
        providerChips(p) +
        '<dl class="provider-facts">' +
          '<div class="provider-fact"><dt>Model</dt><dd><input class="input mono" list="models-' + attr(p.id) + '" data-prov-model="' + attr(p.id) + '" value="' + attr(p.default_model || "") + '" placeholder="choose a model" spellcheck="false" aria-label="Default model for ' + attr(p.label) + '"><datalist id="models-' + attr(p.id) + '">' + opts + "</datalist></dd></div>" +
          '<div class="provider-fact"><dt>API key</dt><dd>' + keyState + "</dd></div>" +
          '<div class="provider-fact"><dt>Model list</dt><dd>' + (models.length ? models.length + " known" : "not fetched") + "</dd></div>" +
          '<div class="provider-fact"><dt>Discovery</dt><dd>' + esc(p.models_from || p.discovery) + "</dd></div>" +
        "</dl>" +
        (result ? '<div class="notice ' + (AI.provBusy[p.id + ":ok"] ? "notice-ok" : "notice-bad") + ' provider-note">' + esc(result) + "</div>" : "") +
      "</div>" +
      '<div class="provider-actions">' +
        '<button type="button" class="btn btn-mini" data-action="prov-test" data-id="' + attr(p.id) + '"' + (busy ? " disabled" : "") + ">Test</button>" +
        '<button type="button" class="btn btn-mini" data-action="prov-models" data-id="' + attr(p.id) + '"' + (busy ? " disabled" : "") + ">Fetch models</button>" +
        '<button type="button" class="btn btn-mini" data-action="prov-verify" data-id="' + attr(p.id) + '"' + (busy || !p.default_model ? " disabled" : "") + ' title="' + attr(p.default_model ? "Run one real request against " + p.default_model + " and force a tool call" : "Choose a model first") + '">Verify model</button>' +
        '<button type="button" class="btn btn-mini" data-action="prov-edit" data-id="' + attr(p.id) + '">Edit</button>' +
        (p.is_default ? "" : '<button type="button" class="btn btn-mini" data-action="prov-default" data-id="' + attr(p.id) + '">Make default</button>') +
        '<button type="button" class="btn btn-mini btn-danger" data-action="prov-remove" data-id="' + attr(p.id) + '">Remove</button>' +
      "</div></article>";
  }

  function providerForm(f) {
    var pr = (AI.presets || []).filter(function (x) { return x.id === f.preset; })[0] || {};
    return '<div class="panel"><div class="panel-head"><span>' + (f.editing ? "Edit " : "Add ") + esc(f.label || pr.label || "provider") + '</span><button type="button" class="btn btn-ghost btn-mini" data-action="prov-cancel">Close</button></div><div class="panel-body two-col">' +
      "<div>" +
      '<div class="field"><label class="field-label" for="pf-id">Id</label><input class="input mono" id="pf-id" data-pf="id" value="' + attr(f.id) + '"' + (f.editing ? " disabled" : "") + ' spellcheck="false"></div>' +
      '<div class="field"><label class="field-label" for="pf-base">Base URL</label><input class="input mono" id="pf-base" data-pf="base_url" value="' + attr(f.base_url) + '" spellcheck="false"><div class="row-sub">https only (plain http is allowed for localhost). The key is sent here, so check it.</div></div>' +
      '<div class="field"><label class="field-label" for="pf-kind">Wire format</label><select class="input" id="pf-kind" data-pf="kind"><option value="openai-compatible"' + (f.kind !== "anthropic" && f.kind !== "openai" ? " selected" : "") + '>OpenAI-compatible chat</option><option value="openai"' + (f.kind === "openai" ? " selected" : "") + ">OpenAI</option><option value=\"anthropic\"" + (f.kind === "anthropic" ? " selected" : "") + ">Anthropic Messages</option></select></div>" +
      '<div class="field"><label class="field-label" for="pf-key">API key' + (f.editing ? " (leave empty to keep the current one)" : "") + '</label><input class="input mono" id="pf-key" type="password" autocomplete="off" data-pf="api_key" value="" placeholder="' + (pr.requires_key === false ? "not needed for local servers" : "paste your key") + '"><div class="row-sub">Stored in a file only you can read, never in the database, logs or responses. Or leave empty and set the environment variable below.</div></div>' +
      '<div class="field"><label class="field-label" for="pf-keyenv">…or read the key from an environment variable</label><input class="input mono" id="pf-keyenv" data-pf="key_env" value="' + attr(f.key_env || "") + '" placeholder="' + attr(pr.key_env || "MY_API_KEY") + '" spellcheck="false"></div>' +
      '<div class="field"><label class="field-label" for="pf-model">Default model</label><input class="input mono" id="pf-model" list="pf-models" data-pf="default_model" value="' + attr(f.default_model || "") + '" spellcheck="false"><datalist id="pf-models">' + modelOptions(f.id).map(function (m) { return '<option value="' + attr(m.model_id) + '"></option>'; }).join("") + "</datalist></div>" +
      '<label class="check"><input type="checkbox" data-pf="make_default"' + (f.make_default ? " checked" : "") + "> Use for new conversations</label>" +
      '<div class="inline" style="margin-top:14px"><button type="button" class="btn btn-primary" data-action="prov-save">' + (f.editing ? "Save" : "Add provider") + "</button></div></div>" +
      '<div><div class="field-label">API setup</div>' +
      (pr.key_url ? '<div class="row-sub">1. Create a key: <a class="link" href="' + attr(pr.key_url) + '" target="_blank" rel="noopener noreferrer">' + esc(pr.key_url) + "</a></div>" : '<div class="row-sub">No key is needed for a local server. Make sure it is running.</div>') +
      '<div class="row-sub">2. Paste it here, or export it: <span class="mono">export ' + esc(pr.key_env || "API_KEY") + "=…</span></div>" +
      '<div class="row-sub">3. Test the connection, fetch the model list, pick a default model.</div>' +
      '<div class="row-sub">Docs: <a class="link" href="' + attr(pr.docs_url || "#") + '" target="_blank" rel="noopener noreferrer">' + esc(pr.docs_url || "") + "</a></div>" +
      '<div class="row-sub">Models come from: ' + esc(pr.models_from || "") + "</div>" +
      (pr.notes ? '<div class="notice" style="margin-top:8px">' + esc(pr.notes) + "</div>" : "") +
      '<div class="field-label" style="margin-top:12px">The request devd sends</div><pre class="log-block">' + esc(curlExample(f.kind, f.base_url || pr.base_url, pr.key_env)) + "</pre></div></div></div>";
  }

  function localPanel() {
    if (!AI.localFound) return "";
    var rows = AI.localFound.map(function (f) {
      var have = (AI.providers || []).some(function (p) { return p.id === f.preset; });
      return '<div class="live"><span class="dot ' + (f.running ? "dot-ok" : "dot-off") + '"></span><span class="grow"><b>' + esc(f.label) + '</b> <span class="dim">' + esc(f.message) + "</span></span>" +
        (f.running && !have ? '<button type="button" class="btn btn-mini btn-primary" data-action="prov-new" data-preset="' + attr(f.preset) + '">Add it</button>' : "") + "</div>";
    }).join("");
    return '<div class="section-title">On this machine</div><div class="panel">' + rows + "</div>";
  }

  /* The catalogue is the models this installation is actually configured to
     use, not a vendor page. A provider contributes its selected default first,
     then the models its own API returned. The official Claude list is only a
     reference, and only when Anthropic is one of the configured providers. */
  function configuredModels() {
    var rows = [];
    (AI.providers || []).forEach(function (p) {
      var seen = {};
      var listed = (AI.catalogue || []).filter(function (m) { return m.source === "provider:" + p.id; });
      if (p.default_model) {
        seen[p.default_model] = 1;
        rows.push({ provider: p, model: { model_id: p.default_model, context_window: null }, isDefault: true, source: "configured default" });
      }
      listed.forEach(function (m) {
        if (!m.model_id || seen[m.model_id]) return;
        seen[m.model_id] = 1;
        rows.push({ provider: p, model: m, isDefault: false, source: "provider list" });
      });
      if (!p.default_model && !listed.length) rows.push({ provider: p, model: null });
    });
    return rows;
  }

  function officialClaudePanel() {
    var official = (AI.catalogue || []).filter(function (m) { return m.source === "anthropic-docs"; });
    var offRows = official.map(function (m) {
      return '<tr><td class="cell-mono">' + esc(m.model_id) + '</td><td class="cell-strong">' + esc(m.display) + "</td><td>" + tok(m.context_window) + "</td><td>" + tok(m.max_output) + "</td><td>" + money(m.input_price) + " / " + money(m.output_price) + '</td><td class="dim">' + esc(m.retirement || "") + "</td>" +
        '<td><button type="button" class="btn btn-mini" data-action="prov-use-model" data-model="' + attr(m.model_id) + '">Use</button></td></tr>';
    }).join("");
    return '<div class="panel"><div class="panel-head"><span>Official Claude models</span><span class="panel-note">' + (official.length ? "fetched " + esc(ago(official[0].fetched_at)) + " from " + '<a class="link" target="_blank" rel="noopener noreferrer" href="https://platform.claude.com/docs/en/about-claude/models/overview">Anthropic docs</a>' : "not fetched yet") + "</span></div>" +
      (offRows ? '<div class="table-wrap"><table class="table"><thead><tr><th>API id</th><th>Name</th><th>Context</th><th>Max output</th><th>$/MTok in / out</th><th>Retirement</th><th></th></tr></thead><tbody>' + offRows + "</tbody></table></div>" : '<div class="panel-body empty-inline">The official list is read from Anthropic\'s documentation and needs no API key.</div>') +
      '<div class="panel-body panel-foot"><button type="button" class="btn btn-primary btn-mini" data-action="models-refresh">' + (official.length ? "Refresh from Anthropic" : "Fetch the official list") + "</button></div></div>";
  }

  function modelPanels() {
    var rows = configuredModels();
    var models = rows.filter(function (row) { return row.model; });
    var body = "";
    if (rows.length) {
      body = '<div class="panel"><div class="panel-head"><span>Configured models</span><span class="panel-note">' + models.length + " model" + (models.length === 1 ? "" : "s") + " from " + (AI.providers || []).length + " provider" + ((AI.providers || []).length === 1 ? "" : "s") + "</span></div>" +
        '<div class="table-wrap"><table class="table"><thead><tr><th>Provider</th><th>Model</th><th>Context</th><th>Source</th><th></th></tr></thead><tbody>' +
        rows.map(function (row) {
          var p = row.provider;
          var label = brandImg(p.preset || p.id, "brand-dot") + esc(p.label) +
            (p.is_default ? ' <span class="pill pill-accent">default provider</span>' : "") +
            (p.enabled === false ? ' <span class="pill">disabled</span>' : "");
          if (!row.model) {
            return '<tr><td><div class="cell-strong">' + label + '</div><div class="cell-dim">' + esc(p.kind) + "</div></td>" +
              '<td colspan="3" class="cell-dim">No model selected. Fetch this provider\'s models, or edit its default.</td>' +
              '<td><button type="button" class="btn btn-mini" data-action="prov-edit" data-id="' + attr(p.id) + '">Edit</button></td></tr>';
          }
          return '<tr><td><div class="cell-strong">' + label + '</div><div class="cell-dim">' + esc(p.kind) + "</div></td>" +
            '<td class="cell-mono">' + esc(row.model.model_id) + "</td>" +
            "<td>" + tok(row.model.context_window) + "</td>" +
            '<td class="dim">' + esc(row.source) + "</td>" +
            "<td>" + (row.isDefault ? '<span class="pill pill-accent">default</span>' : '<button type="button" class="btn btn-mini" data-action="prov-pick" data-id="' + attr(p.id) + '" data-model="' + attr(row.model.model_id) + '">Use as default</button>') + "</td></tr>";
        }).join("") +
        "</tbody></table></div></div>";
    } else {
      body = uiEmpty("No provider configured", "Add a provider below, then fetch its models. The catalogue shows what this installation can actually use.");
    }
    var anthropic = (AI.providers || []).some(function (p) { return p.kind === "anthropic"; });
    if (anthropic) {
      body += '<details class="ui-fold"><summary>Official Claude reference</summary>' + officialClaudePanel() + "</details>";
    }
    return body;
  }

  function renderProviders() {
    if (state.view !== "providers") return;
    if (!AI.providers) return renderLoading();
    var cards = AI.providers.map(providerCard).join("");
    var presetCards = (AI.presets || []).map(function (pr) {
      var cat = pr.category === "local" ? "local" : pr.category === "aggregator" ? "aggregator" : "cloud";
      return '<button type="button" class="preset" data-action="prov-new" data-preset="' + attr(pr.id) + '">' +
        '<span class="preset-head">' + brandImg(pr.id, "brand-dot") + "<b>" + esc(pr.label) + '</b><span class="preset-cat">' + esc(cat) + "</span></span>" +
        "<span>" + esc(pr.requires_key ? "Needs an API key" : "No key") + " · " + esc(pr.tools === "documented" ? "tool calling" : "tools depend on the model") + "</span>" +
        "<small>" + esc(pr.models_from || "") + "</small></button>";
    }).join("");
    var empty = '<div class="panel"><div class="empty"><div class="empty-icon"><svg class="icon" aria-hidden="true"><use href="#i-sliders"></use></svg></div>' +
      '<div class="empty-title">No model provider yet</div>' +
      '<div class="empty-hint">Connect Anthropic, OpenAI, DeepSeek, Kimi, GLM, Gemini, OpenRouter, a local Ollama instance, or any OpenAI-compatible endpoint. Keys are stored privately and never returned by the API.</div>' +
      '<div class="empty-actions"><a class="btn btn-primary" href="#add-provider">Choose a provider</a></div></div></div>';

    var roles = AI.providers.length ? rolesPanel() : "";
    el("main").innerHTML = page("Models", "",
      '<button type="button" class="btn" data-action="prov-discover">Find local servers</button>',
      (AI.providers.length ? modelsInUse() : "") +
      (cards ? uiHead("Providers", AI.providers.length) + '<div class="provider-list">' + cards + "</div>" : empty) +
      (AI.provForm ? providerForm(AI.provForm) : "") +
      localPanel() +
      (roles ? '<details class="ui-fold"><summary>Which model does which work</summary>' + roles + "</details>" : "") +
      uiHead("Add a provider", null) + '<div class="preset-grid" id="add-provider">' + presetCards + "</div>" +
      '<details class="ui-fold"><summary>Model catalogue</summary>' + modelPanels() + "</details>");
  }

  // --------------------------------------------------------------- assistant

  function loadAssistant() {
    return Promise.all([read("chat.list"), loadProvidersIfNeeded(), state.projects.length ? Promise.resolve() : loadProjects()]).then(function (r) {
      AI.threads = r[0] || [];
      if (AI.thread) return pollThread(true);
    });
  }
  function loadProvidersIfNeeded() { return AI.providers && AI.catalogue ? Promise.resolve() : loadProviders(); }

  /* One sentence for a screen reader when something new arrives. The stream
     itself is a log that is re-rendered on every poll, so it deliberately does
     not announce its whole contents again. */
  function announceEvent(e) {
    var text = "";
    if (e.kind === "assistant") {
      text = (e.data.content || [])
        .filter(function (b) { return b.type === "text"; })
        .map(function (b) { return b.text; })
        .join(" ");
    } else if (e.kind === "error") {
      text = e.data.message || "The assistant failed.";
    } else if (e.kind === "notice") {
      text = e.data.text || "";
    }
    if (!text) return;
    var node = el("announcer");
    if (node) node.textContent = "devd: " + clip(text, 300);
  }

  function pollThread(full) {
    if (!AI.thread) return Promise.resolve();
    return read("chat.get", { thread: AI.thread.id, after: full ? 0 : AI.lastId }).then(function (d) {
      if (full) { AI.events = []; AI.lastId = 0; }
      d.events.forEach(function (e) {
        // Two polls can overlap (the timer and the live channel, or a click and
        // an event). Whichever answers second must not append what the first
        // already did: duplicate events are duplicate replies on screen.
        if (e.id <= AI.lastId) return;
        AI.events.push(e);
        AI.lastId = e.id;
        if (!full) announceEvent(e);
      });
      AI.thread = d.thread; AI.pending = d.pending || []; AI.running = d.running; AI.chapters = d.chapters || []; AI.run = d.run || null;
      if (d.thread.provider_id) AI.pickProvider = d.thread.provider_id;
      if (d.thread.model) AI.pickModel = d.thread.model;
      AI.queued = d.queued || 0;
      var idx = (AI.threads || []).map(function (t) { return t.id; }).indexOf(d.thread.id);
      if (idx >= 0) AI.threads[idx] = d.thread;
      // The plan the agent wrote is project state, not conversation state. Read it when a thread
      // opens and every few seconds after, so it stays true while the person works elsewhere.
      var project = d.thread.project;
      if (project && Date.now() - (AI.todosAt || 0) > 5000) {
        AI.todosAt = Date.now();
        read("todo.list", { project: project }).then(function (list) {
          AI.todos = Array.isArray(list) ? list : [];
          paintChatPlan();
        }).catch(function () { /* the plan is optional */ });
      }
    });
  }

  function startThreadTimer() {
    clearInterval(AI.timer);
    // The stream is the update; the timer is only a safety net for a browser
    // that cannot hold an EventSource open, and it runs far less often than the
    // 800 ms it used to.
    AI.timer = setInterval(function () {
      if (state.view !== "assistant") return clearInterval(AI.timer);
      if (!AI.thread) return;
      var busy = AI.thread.state === "running" || AI.running;
      pollThread(false).then(function () {
        var was = busy, now = AI.thread.state === "running" || AI.running;
        paintChat();
        if (was && !now) { read("chat.list").then(function (t) { AI.threads = t; paintThreads(); }); loadHealth().then(healthChip); }
      }).catch(function () {});
    }, AI.live ? 20000 : 800);
    openLive();
  }

  /* The live channel.
   *
   * Everything a working agent does is published on the server's event bus:
   * a reply being written (`chat.delta`), a step starting or finishing
   * (`chat.event`). This subscribes once for the whole window instead of
   * asking the server whether anything changed, so the text appears as it is
   * written rather than in a burst when the turn ends.
   *
   * The event stream is a hint, never the record: on open, on reconnect and on
   * every event the conversation re-reads `chat.get` past the last id it has.
   * The deltas are what make it look alive between those reads. */
  function openLive() {
    if (AI.live) return;
    if (typeof window.EventSource !== "function") return;
    var source;
    try {
      source = new window.EventSource("/api/events");
    } catch (e) {
      return;
    }
    AI.live = source;
    // Durable run events have stable ids and are replayed from Last-Event-ID by
    // the server, so a reconnect reconstructs the timeline instead of silently
    // losing frames. Live text remains a non-durable hint.
    source.addEventListener("run", function (message) {
      var event;
      try { event = JSON.parse(message.data); } catch (e) { return; }
      onDurable(event);
    });
    source.addEventListener("live", function (message) {
      var event;
      try { event = JSON.parse(message.data); } catch (e) { return; }
      onLive(event);
    });
    // Older servers named the one stream "devd"; keep working with one.
    source.addEventListener("devd", function (message) {
      var event;
      try { event = JSON.parse(message.data); } catch (e) { return; }
      onLive(event);
    });
    source.onopen = function () {
      // The durable replay closes the history gap; the conversation itself is
      // re-read past the last event it already has.
      if (!AI.thread) return;
      pollThread(false).then(paintChat).catch(function () {});
    };
    source.onerror = function () {
      // The browser retries by itself; the safety-net poll keeps the view
      // correct until it does.
      if (!AI.thread) return;
      pollThread(false).then(paintChat).catch(function () {});
    };
  }

  /* A durable run event. It is a notification, not the record: the run's own
   * endpoints are re-read so the view never depends on having seen every frame. */
  function onDurable(event) {
    if (!event || !event.run) return;
    // A lag in the live text is not a gap in the durable timeline: the
    // conversation is re-read from the log, which is the record.
    if (AI.thread && AI.run && AI.run.id === event.run) {
      pollThread(false).then(paintChat).catch(function () {});
    }
    if (String(event.kind || "").indexOf("approval.") === 0) {
      loadHealth().then(healthChip).catch(function () {});
    }
  }

  function onLive(event) {
    if (!event || !event.data) return;
    var data = event.data;
    if (data.thread && (!AI.thread || data.thread !== AI.thread.id)) return;
    if (!AI.thread) return;
    if (event.kind === "chat.delta") {
      if (state.view !== "assistant") return;
      if (data.start) {
        // A new model call (or a new provider after a failure) begins. The
        // previous call's text is about to be logged; hide it now so the new
        // words do not appear underneath it.
        clearStreamTail(true);
        return;
      }
      var piece = String(data.text || "");
      if (!piece) return;
      AI.streamTail += piece;
      scheduleStreamPaint();
      return;
    }
    // A reply has been logged, a step finished, or the turn ended. The record
    // is the truth; one read and one render answer for it. When the reply is
    // logged its streamed copy has to go before that render, or the same words
    // appear twice: once as the persisted bubble and once as the live tail.
    var done = event.kind === "chat.event" && (data.kind === "assistant" || data.kind === "error" || data.kind === "notice");
    if (done) clearStreamTail(false);
    pollThread(false).then(function () {
      paintChat();
      if (state.view === "assistant") loadHealth().then(healthChip);
    }).catch(function () {});
  }

  /* The text of the turn being written. It is one bubble, filled as tokens
     arrive; when the reply is logged this tail is cleared and the persisted
     bubble takes exactly the same place. */
  function paintStreamTail() {
    var text = AI.streamTail || "";
    var typing = el("chat-typing");
    var box = el("chat-tail");
    if (typing) typing.hidden = !!text;
    if (!box) return;
    var stream = el("chat-stream");
    // Follow the bottom only when the person is already reading there.
    var near = stream ? stream.scrollHeight - stream.scrollTop - stream.clientHeight < 90 : false;
    box.hidden = !text;
    if (AI.streamPainted !== text) {
      box.innerHTML = text ? '<div class="bubble">' + md(text) + "</div>" : "";
      AI.streamPainted = text;
    }
    if (stream && near) stream.scrollTop = stream.scrollHeight;
  }

  /* Tokens can arrive faster than a screen can usefully repaint. Coalescing
     them into one frame keeps the text smooth without re-parsing markdown for
     every character. */
  function scheduleStreamPaint() {
    if (AI.streamFrame) return;
    var run = function () { AI.streamFrame = 0; paintStreamTail(); };
    AI.streamFrame = window.requestAnimationFrame ? window.requestAnimationFrame(run) : window.setTimeout(run, 50);
  }

  function stopStreamPaint() {
    if (!AI.streamFrame) return;
    if (window.cancelAnimationFrame) window.cancelAnimationFrame(AI.streamFrame);
    else window.clearTimeout(AI.streamFrame);
    AI.streamFrame = 0;
  }

  function clearStreamTail(paintNow) {
    AI.streamTail = "";
    AI.streamPainted = "";
    stopStreamPaint();
    // When the reply is about to be re-read from the record, the existing
    // bubble already holds the same words; leaving it until the render avoids
    // a flash of the working dots between the two.
    if (paintNow) paintStreamTail();
  }

  /* What a tool call was, in one line.
   *
   * The detail is what a person would say, not the raw arguments: "the file
   * it edited", "the command it ran". The full arguments are one click away. */
  function summarizeCall(b) {
    var i = b.input || {};
    if (b.name === "devd_invoke") {
      var id = i.id || "invoke";
      var a = i.args || {};
      return { title: id, detail: callDetail(id, a) };
    }
    if (b.name === "devd_describe") return { title: "describe " + (i.id || ""), detail: "" };
    if (b.name === "devd_capabilities") return { title: "search capabilities", detail: [i.group, i.query].filter(Boolean).join(" ") };
    if (b.name === "devd_groups") return { title: "list domains", detail: "" };
    return { title: b.name, detail: "" };
  }

  /* The one-line detail of a capability call.
   *
   * For a project command the command is the point, not the folder it ran
   * in: "npm run test" says what happened, "." does not. */
  function callDetail(id, a) {
    if (!a || typeof a !== "object") return "";
    var project = /^project\./.test(String(id || ""));
    if (project && a.task) return String(a.task);
    if (project && Array.isArray(a.argv)) return a.argv.join(" ");
    if (a.path) return String(a.path);
    if (a.task) return String(a.task);
    if (Array.isArray(a.argv)) return a.argv.join(" ");
    if (a.query) return String(a.query);
    if (a.ref) return String(a.ref);
    if (a.url) return String(a.url);
    if (a.name) return String(a.name);
    var keys = Object.keys(a);
    if (!keys.length) return "";
    return keys
      .slice(0, 2)
      .map(function (k) { return k + "=" + JSON.stringify(a[k]); })
      .join(" ");
  }

  /* How long an action took, in the words people use. */
  function duration(ms) {
    if (!ms || ms < 0) return "";
    if (ms < 1000) return Math.round(ms) + "ms";
    if (ms < 60000) return (ms / 1000).toFixed(1) + "s";
    return Math.round(ms / 60000) + "m";
  }

  /* The name of a capability, as a person would say it.
   *
   * The capability ids are exact and machine-shaped ("fs.edit",
   * "project.exec"). A person reading a conversation wants to know what the
   * agent did, not what it called. */
  var CAPABILITY_NAME = {
    "fs.read": "Read",
    "fs.write": "Write",
    "fs.edit": "Edit",
    "fs.list": "List",
    "fs.search": "Search",
    "fs.delete": "Delete",
    "fs.move": "Move",
    "fs.browse": "Browse",
    "project.exec": "Shell",
    "project.run": "Run",
    "project.tasks": "Tasks",
    "project.detect": "Detect",
    "git.status": "Status",
    "git.log": "Log",
    "git.diff": "Diff",
    "git.commit": "Commit",
  };

  function capabilityName(id) {
    if (!id) return "Tool";
    if (CAPABILITY_NAME[id]) return CAPABILITY_NAME[id];
    var tail = String(id).split(".").pop() || String(id);
    return tail.charAt(0).toUpperCase() + tail.slice(1);
  }

  /* What a tool call was, in words.
   *
   * The agent calls one generic tool, "devd_invoke", and passes it the
   * capability to run. The capability is the interesting part, so that is
   * what the name is built from. */
  function toolLabel(block) {
    var i = block.input || {};
    if (block.name === "devd_invoke") return capabilityName(i.id);
    if (block.name === "devd_describe") return "Describe";
    if (block.name === "devd_capabilities") return "Search";
    if (block.name === "devd_groups") return "Domains";
    return capabilityName(block.name);
  }

  /* "Used 4 Edit, Shell, Read" - what the agent did before it spoke.
   *
   * The number counts the calls; the names are the distinct ones, so three
   * edits read as "Used 3 Edit" and a mix reads as "Used 4 Edit, Shell". */
  function usedLine(blocks) {
    var calls = 0;
    var names = [];
    (blocks || []).forEach(function (b) {
      if (b.type !== "tool_use") return;
      calls += 1;
      var name = toolLabel(b);
      if (names.indexOf(name) === -1) names.push(name);
    });
    if (!calls) return "";
    return '<div class="msg-used">Used ' + calls + " " + names.join(", ") + "</div>";
  }

  /* The row under a reply. Copy takes the reply's text; Fork starts a new conversation from it and
   * leaves this one as it is; Pin marks it as a chapter of this conversation. */
  function msgActions(eventId, block) {
    var chapter = (AI.chapters || []).filter(function (c) { return c.event_id === eventId; })[0];
    var at = ' data-event="' + eventId + '" data-block="' + block + '"';
    return '<div class="msg-actions">' +
      '<button type="button" class="msg-act" data-action="msg-copy"' + at + ' title="Copy this reply"><svg class="icon" aria-hidden="true"><use href="#i-copy"></use></svg>Copy</button>' +
      '<button type="button" class="msg-act" data-action="msg-fork"' + at + ' title="Start a new conversation from this point. This one stays as it is"><svg class="icon" aria-hidden="true"><use href="#i-branch"></use></svg>Fork from here</button>' +
      (chapter
        ? '<button type="button" class="msg-act msg-act-on" data-action="msg-pin"' + at + ' title="Remove this chapter"><svg class="icon" aria-hidden="true"><use href="#i-pin"></use></svg>Pinned: ' + esc(clip(chapter.title, 40)) + "</button>"
        : '<button type="button" class="msg-act" data-action="msg-pin"' + at + ' title="Mark this reply as a chapter of the conversation"><svg class="icon" aria-hidden="true"><use href="#i-pin"></use></svg>Pin as chapter</button>') +
      "</div>";
  }

  function chatHtml() {
    var results = {};
    var resultAt = {};
    AI.events.forEach(function (e) {
      if (e.kind !== "tool_result") return;
      results[e.data.tool_use_id] = e.data;
      resultAt[e.data.tool_use_id] = e.created_at;
    });
    var pendingByTool = {};
    AI.pending.forEach(function (r) { pendingByTool[r.tool_use_id] = r; });

    /* A reply is one turn, not one bubble per model call.
     *
     * The model speaks more than once per answer: a sentence of intent, then a
     * call, then the real reply. Every one of those calls used to become its
     * own message with its own avatar and timestamp, which is why a single
     * question produced "I'll check the project's files first." and then the
     * answer as if two assistants had spoken. A turn is the span from a person's
     * message to the next one; its steps and its text are arranged in the order
     * the agent produced them, under one heading. */
    var blocks = []; // the conversation in order: a person's message, then one assistant turn
    var current = null; // the assistant turn being built, if any
    var activity = ""; // an open run of steps in that turn
    // Steps stay in one group until something that is not a step arrives.
    var closeGroup = function () {
      if (!activity) return;
      activity += "</div>";
      if (current) current.items.push(activity);
      activity = "";
    };
    var openTurn = function (at) {
      current = { at: at || "", items: [], copy: null, last: "none" };
      blocks.push({ type: "turn", turn: current });
    };
    var endTurn = function () {
      if (!current) return;
      closeGroup();
      current = null;
    };

    AI.events.forEach(function (e) {
      if (e.kind === "user") {
        // A person's message stands on its own. The assistant's reply is the
        // turn that follows it; it does not share the person's row or avatar.
        endTurn();
        blocks.push({ type: "user", at: e.created_at, text: e.data.text || "" });
        return;
      }
      if (e.kind === "assistant") {
        if (!current) openTurn(e.created_at);
        // Each model call's tool run gets its own count. Flushing here keeps
        // "Used N" honest when a turn makes several tool calls in a row across
        // model calls; they stay in one turn because the turn is the container.
        closeGroup();
        (e.data.content || []).forEach(function (b) {
          if (b.type === "text" && b.text.trim()) {
            // The first words of a turn that has only run tools so far are the
            // turn's opening line, not a caption on the steps below it: they sit
            // above them, as the assistant speaking before it works. The row
            // under a turn acts on the turn: copy, fork, pin.
            closeGroup();
            current.items.push('<div class="bubble" data-event="' + e.id + '">' + md(b.text) + "</div>");
            current.copy = e.id;
            current.last = "text";
          } else if (b.type === "tool_use") {
            var c = summarizeCall(b), r = results[b.id], open = AI.openSteps[b.id];
            var pend = pendingByTool[b.id];
            var out = r ? clip(String(r.content == null ? "" : r.content), 20000) : "";
            var took = "";
            if (r && resultAt[b.id] && e.created_at) {
              var ms = Date.parse(resultAt[b.id]) - Date.parse(e.created_at);
              if (ms > 0 && ms < 3600000) took = duration(ms);
            }
            // Consecutive actions read as one run with its own count, inside the
            // turn: "Used 2 Read, List" and the two rows are one block.
            if (!activity) activity = '<div class="msg-tools">' + usedLine(e.data.content);
            current.last = "tools";
            activity += '<div class="step' + (r && r.is_error ? " step-bad" : "") + (open ? " open" : "") + '">' +
              '<button type="button" class="step-head" data-action="step-toggle" data-id="' + attr(b.id) + '" aria-expanded="' + (open ? "true" : "false") + '" title="' + attr(c.title + (c.detail ? " — " + c.detail : "")) + '">' +
              '<span class="step-arrow" aria-hidden="true"><svg class="icon"><use href="#i-chevron-down"></use></svg></span>' +
              '<span class="step-dot ' + (pend ? "wait" : r ? (r.is_error ? "bad" : "ok") : "run") + '" aria-hidden="true"></span>' +
              '<span class="step-title">' + esc(c.title) + "</span>" +
              '<span class="step-detail">' + esc(c.detail) + "</span>" +
              (took ? '<span class="step-time">' + esc(took) + "</span>" : "") +
              "</button>" +
              (open
                ? '<div class="step-body"><div class="field-label">Input</div><pre class="log-block">' + esc(JSON.stringify(b.input, null, 2)) + "</pre>" +
                  (r
                    ? '<div class="inline"><div class="field-label grow">Result</div><button type="button" class="btn btn-mini btn-ghost" data-action="copy-output" data-id="out-' + attr(b.id) + '">Copy</button></div><pre class="log-block" id="out-' + attr(b.id) + '">' + esc(out) + "</pre>"
                    : "") +
                  "</div>"
                : "") +
              "</div>";
          }
        });
        return;
      }
      if (!current) openTurn(e.created_at);
      if (e.kind === "approval") {
        closeGroup();
        var req = AI.pending.filter(function (r) { return r.id === e.data.request_id; })[0];
        if (req) current.items.push(approvalCard(req, false));
        current.last = "note";
      } else if (e.kind === "notice") {
        closeGroup();
        current.items.push('<div class="msg-note">' + esc(e.data.text) + "</div>");
        current.last = "note";
      } else if (e.kind === "error") {
        closeGroup();
        current.items.push('<div class="msg-note msg-error">' + esc(e.data.message) + "</div>");
        current.last = "note";
      }
    });

    endTurn();

    var working = AI.thread && (AI.thread.state === "running" || AI.running);
    // The moment a person sends, before the first assistant event exists, there
    // is still a reply coming: show it as an empty turn with the working dots.
    if (working && (!blocks.length || blocks[blocks.length - 1].type === "user")) {
      blocks.push({ type: "turn", turn: { at: "", items: [], copy: null, last: "none" } });
    }

    var html = blocks.map(function (block, i) {
      if (block.type === "user") {
        var when = shortTime(block.at);
        return '<div class="msg msg-user"><div><div class="bubble">' + md(block.text) + '</div><div class="msg-meta">You' + (when ? " · " + when : "") + "</div></div></div>";
      }
      var turn = block.turn;
      var active = i === blocks.length - 1 && working;
      var body = turn.items.join("");
      // The working dots are the wait before the first word (or while a tool
      // runs). Once text has been written they give way to the text itself.
      var typing = active && turn.last !== "text"
        ? '<div class="bubble typing typing-inline" id="chat-typing" role="status" aria-label="devd is working"><span></span><span></span><span></span></div>'
        : "";
      // The text being written is placed here; paintStreamTail fills it. It is
      // an overlay so a token does not re-render the whole conversation.
      var tail = active ? '<div class="msg-tail" id="chat-tail" hidden></div>' : "";
      return '<div class="turn msg-agent">' +
        '<div class="avatar" aria-hidden="true">d</div>' +
        '<div class="turn-body">' + body + typing + tail +
        (body || typing || tail ? '<div class="msg-meta">devd' + (shortTime(turn.at) ? " · " + shortTime(turn.at) : "") + "</div>" : "") +
        (turn.copy && !active ? msgActions(turn.copy, 0) : "") +
        "</div></div>";
    }).join("");

    // A conversation with no turns yet. It is not a greeting: it says what the
    // assistant is working with, from what devd knows, and offers real starting points.
    if (html) return html;
    var provs = (AI.providers || []).filter(function (p) { return p.enabled; });
    var prov = provs.filter(function (p) { return p.id === (AI.thread && AI.thread.provider_id) || p.id === AI.pickProvider; })[0] ||
      provs.filter(function (p) { return p.is_default; })[0] || provs[0];
    var projPath = AI.project || "";
    var projName = projPath ? ((state.projects || []).filter(function (x) { return x.path === projPath; })[0] || {}).name || dirName(projPath) : "";
    var host = ""; try { host = prov && prov.base_url ? new URL(prov.base_url).host : ""; } catch (e) { host = ""; }
    var where = !prov ? "" : prov.category === "local" ? "runs on this machine" : "remote" + (host ? ": " + host : "");
    var auto = state.session && state.session.settings && state.session.settings.agent_auto_approve_write;
    var starts = projPath
      ? [["Review my changes", "Review the uncommitted changes in this project and tell me what needs attention."],
         ["Run the tests and fix what fails", "Run the project's tests and checks, then fix what actually fails."],
         ["Explain how this project is built", "Explain how this project is structured: the entry points, the main modules and how they connect."]]
      : [["What needs my attention", "What needs my attention right now?"],
         ["Why can't one of my sites serve", "Which of my sites cannot serve, and why?"],
         ["Start a new project", "Help me create a new project. Ask me what I am building first."]];
    return '<div class="assist-empty">' +
      '<h2 class="assist-title">What are we working on?</h2>' +
      '<dl class="assist-ctx">' +
        '<div><dt>Project</dt><dd>' + (projName ? esc(projName) : '<span class="dim">none selected - choose one below to give it a codebase</span>') + "</dd></div>" +
        '<div><dt>Model</dt><dd>' + (prov ? esc(prov.label) + (prov.default_model ? ' <span class="mono">' + esc(prov.default_model) + "</span>" : "") + ' <span class="dim">' + esc(where) + "</span>" : '<span class="dim">none connected</span>') + "</dd></div>" +
        '<div><dt>Changes</dt><dd>' + (auto ? "Writes files without asking" : "Checks with you before it writes files or does anything hard to undo") + ' <span class="dim">- every call is recorded in Activity</span></dd></div>' +
      "</dl>" +
      '<div class="assist-starts" role="group" aria-label="Ways to start"><div class="assist-label">Start with</div>' +
        starts.map(function (x) { return '<button type="button" class="assist-start" data-action="chat-suggest" data-prompt="' + attr(x[1]) + '"><span>' + esc(x[0]) + '</span><svg class="icon" aria-hidden="true"><use href="#i-chevron"></use></svg></button>'; }).join("") +
      "</div></div>";
  }

  /* Everything the conversation's markup depends on.
   *
   * The stream used to be rebuilt from scratch every 800ms, changed or not. A full innerHTML
   * replacement drops the browser's scroll anchor, so while a person was reading further up the
   * conversation - or selecting text - the view crept and jumped under them. Painting only when this
   * key changes, and putting the scroll position back exactly, makes the idle case cost nothing and
   * the busy case keep the reader's place. */
  /* The plan, above the conversation. The agent writes it with todo.create; this is the same
   * project state the person can see in Agents → Notes, not a private scratch list. */
  function paintChatPlan() {
    var box = el("chat-plan");
    if (!box) return;
    var all = AI.todos || [];
    var open = all.filter(function (t) { return t.status !== "done" && t.status !== "cancelled"; });
    if (!open.length) { box.hidden = true; box.innerHTML = ""; return; }
    var done = all.filter(function (t) { return t.status === "done"; }).length;
    box.hidden = false;
    box.innerHTML = '<span class="chat-plan-label">Plan</span><span class="chat-plan-count">' + done + "/" + all.length + "</span>" +
      open.slice(0, 3).map(function (t) { return '<span class="chat-plan-item" title="' + attr(t.title) + '">' + esc(t.title) + "</span>"; }).join("") +
      (open.length > 3 ? '<span class="dim">+' + (open.length - 3) + "</span>" : "") +
      '<button type="button" class="btn btn-mini btn-ghost" data-action="chat-plan">View</button>';
  }

  function chatPaintKey() {
    var t = AI.thread || {};
    return [
      AI.events.length,
      AI.lastId || 0,
      AI.events.length ? AI.events[0].id : "",
      t.id || "", t.state || "", t.title || "", t.project || "", t.input_tokens || 0, t.output_tokens || 0,
      AI.running ? "1" : "0",
      AI.queued || 0,
      (AI.pending || []).map(function (r) { return r.id + (r.state || ""); }).join(","),
      (AI.chapters || []).map(function (c) { return c.event_id + ":" + c.title; }).join(","),
      Object.keys(AI.openSteps || {}).filter(function (k) { return AI.openSteps[k]; }).sort().join(",")
    ].join("|");
  }

  function paintChat(force) {
    var box = el("chat-stream");
    if (!box) return;
    var key = chatPaintKey();
    if (!force && key === AI.paintKey) return;
    // A text selection lives in the DOM; replacing it just to redraw the same conversation is rude.
    var sel = window.getSelection ? window.getSelection() : null;
    if (!force && sel && sel.rangeCount && !sel.isCollapsed && sel.anchorNode && box.contains(sel.anchorNode)) return;
    AI.paintKey = key;
    var busy = AI.thread && (AI.thread.state === "running" || AI.running);
    var tid = (AI.thread && AI.thread.id) || "";
    if (AI.streamThread !== tid) {
      AI.streamThread = tid;
      AI.streamTail = "";
      AI.streamPainted = "";
      stopStreamPaint();
    }
    if (!busy) {
      AI.streamTail = "";
      AI.streamPainted = "";
      stopStreamPaint();
    }
    var near = box.scrollHeight - box.scrollTop - box.clientHeight < 80;
    var top = box.scrollTop;
    box.setAttribute("aria-busy", busy ? "true" : "false");
    box.innerHTML = chatHtml();
    // The render replaced the tail element with a fresh empty one.
    AI.streamPainted = "";
    // Follow the bottom only when the person was already there; otherwise stay exactly where they were.
    box.scrollTop = near ? box.scrollHeight : top;
    // The text of the turn being written is an overlay on the last turn; a
    // re-render rebuilds the boxes under it, so it is laid down again.
    paintStreamTail();
    var t = AI.thread;
    var head = el("chat-head");
    if (head && t) head.innerHTML = chatHeadHtml();
    var send = el("chat-send");
    if (send) {
      // The arrow always sends or queues. While it works, the square beside it stops; a message
      // written now is kept for the next step instead of being refused.
      send.innerHTML = '<svg class="icon" aria-hidden="true"><use href="#i-arrow-up"></use></svg>';
      send.setAttribute("data-action", "chat-send");
      send.setAttribute("aria-label", busy ? "Queue this message" : "Send");
      send.setAttribute("title", busy ? "Queue this message for the next step" : "Send");
      send.className = "composer-send";
      var stop = el("chat-stop");
      if (stop) stop.hidden = !busy;
    }
    syncComposerAction();
  }

  function chatHeadHtml() {
    var t = AI.thread;
    var r = AI.run;
    // The provider and the model live in the composer, where they are used.
    // The head says what the conversation is and where it stands.
    return '<div class="grow"><div class="chat-title">' + esc(t.title) + '</div><div class="dim mono nowrap">' + esc(t.project || "no project") + " · " + t.input_tokens + " in / " + t.output_tokens + " out tokens</div></div>" +
      (AI.queued > 0 ? '<span class="pill pill-warn" title="Messages waiting for the next step">' + AI.queued + " queued</span>" : "") +
      (AI.chapters && AI.chapters.length ? '<button type="button" class="btn btn-mini btn-ghost" data-action="chapters-open">Chapters (' + AI.chapters.length + ")</button>" : "") +
      (r ? '<button type="button" class="btn btn-mini btn-ghost" data-action="open-run" data-id="' + attr(r.id) + '" title="See this conversation as a run: steps, capability calls, evidence and the durable timeline">' + runPill(r.state) + "</button>" : "") +
      (r && !(t.state === "running" || AI.running) && runCanRetry(r.state) ? '<button type="button" class="btn btn-mini btn-primary" data-action="run-retry" data-id="' + attr(r.id) + '">Continue</button>' : "") +
      (r ? "" : '<span class="pill ' + (t.state === "error" ? "pill-bad" : t.state === "idle" ? "" : "pill-accent") + '">' + esc(t.state.replace("_", " ")) + '</span>') +
      '<button type="button" class="btn btn-mini btn-ghost" data-action="chat-delete" data-id="' + attr(t.id) + '">Delete</button>';
  }

  function paintThreads() {
    var box = el("thread-list");
    if (!box) return;
    box.innerHTML = (AI.threads || []).map(function (t) {
      return '<div class="thread-row"><button type="button" class="thread' + (AI.thread && AI.thread.id === t.id ? " active" : "") + '" data-action="open-thread" data-id="' + attr(t.id) + '"><span class="dot ' + (t.state === "running" ? "dot-ok" : t.state === "waiting_approval" ? "dot-warn" : t.state === "error" ? "dot-bad" : "dot-off") + '"></span><span class="grow nowrap">' + esc(t.title) + '</span><span class="dim">' + esc(ago(t.updated_at)) + '</span></button><button type="button" class="thread-del" data-action="chat-delete" data-id="' + attr(t.id) + '" title="Delete this conversation" aria-label="Delete this conversation">\u00d7</button></div>';
    }).join("") || '<div class="empty-inline">No conversations yet.</div>';
  }

  function renderAssistant() {
    if (state.view !== "assistant") return;
    if (!AI.threads) return renderLoading();
    var ready = (AI.providers || []).some(function (p) { return p.enabled && (p.has_key || !p.needs_key); });
    var projOpts = '<option value="">No project</option>' + state.projects.map(function (p) { return '<option value="' + attr(p.path) + '"' + (AI.project === p.path ? " selected" : "") + ">" + esc(p.name || dirName(p.path)) + "</option>"; }).join("") +
      (DEV.projects || []).map(function (p) { return '<option value="' + attr(p.path) + '"' + (AI.project === p.path ? " selected" : "") + ">" + esc(p.name) + "</option>"; }).join("");

    // The model picker lives in the composer, where it is used, rather than in
    // a header above the conversation.
    var provs = (AI.providers || []).filter(function (p) { return p.enabled; });
    var cur = (AI.thread && AI.thread.provider_id) || AI.pickProvider ||
      (provs.filter(function (p) { return p.is_default; })[0] || provs[0] || {}).id;
    var current = provs.filter(function (p) { return p.id === cur; })[0];
    var currentModel = (AI.thread && AI.thread.model) || AI.pickModel || (current && current.default_model) || "";
    var models = modelOptions(cur);
    if (!currentModel && models.length) currentModel = models[0].model_id;
    if (!AI.thread && !AI.pickModel && currentModel) AI.pickModel = currentModel;
    var known = {};
    var currentLabel = currentModel;
    var modelRows = models.map(function (m) {
      known[m.model_id] = true;
      var label = m.display || m.model_id;
      if (m.model_id === currentModel) currentLabel = label;
      return '<option value="' + attr(m.model_id) + '" title="' + attr(label + (m.model_id !== label ? " (" + m.model_id + ")" : "")) + '"' + (m.model_id === currentModel ? " selected" : "") + ">" + esc(label) + "</option>";
    }).join("");
    if (currentModel && !known[currentModel]) {
      modelRows = '<option value="' + attr(currentModel) + '" selected>' + esc(currentModel) + (models.length ? " (not in provider list)" : "") + "</option>" + modelRows;
    }
    if (!modelRows) {
      modelRows = '<option value="' + attr(currentModel) + '" selected>' + (currentModel ? esc(currentModel) : "provider default") + "</option>";
    }
    var projectName = AI.project || (state.project && state.project.path) || "";

    var composer =
      '<div class="composer">' +
      '<div class="mention-menu" id="chat-mentions" hidden role="listbox" aria-label="Reference"></div>' +
      composerTop("chat") +
      '<textarea class="composer-input" id="chat-input" rows="2" placeholder="What are we working on? Type / or @ to bring in a skill, a tool or a project" aria-label="Message" aria-autocomplete="list" aria-controls="chat-mentions">' + esc(AI.composer) + "</textarea>" +
      '<div class="composer-bar">' +
      '<div class="composer-left">' +
      (AI.thread ? "" : modeSelect() + '<label class="composer-where"><svg class="icon" aria-hidden="true"><use href="#i-folder"></use></svg><span class="sr-only">Project</span><select class="select-field" id="chat-project" aria-label="Project">' + projOpts + "</select></label>") +
      attachButton("chat") +
      '<select class="composer-picker select-field" data-chat="provider" aria-label="Provider">' +
      (provs.length ? provs.map(function (p) { return '<option value="' + attr(p.id) + '"' + (p.id === cur ? " selected" : "") + ">" + esc(p.label) + "</option>"; }).join("") : '<option value="">no provider</option>') +
      "</select>" +
      '<select class="composer-picker composer-model select-field" data-chat="model" aria-label="Model" title="' + attr(currentLabel + (currentModel && currentLabel !== currentModel ? " (" + currentModel + ")" : "")) + '">' + modelRows + "</select>" +
      "</div>" +
      '<div class="composer-right">' +
      // The slot holds exactly one primary action. While it works and the box
      // is empty that is Stop, in the send button's place; once there is text
      // it is the arrow, which queues that text for the next step.
      '<span class="composer-slot">' +
      '<button type="button" class="composer-stop" id="chat-stop" data-action="chat-stop" title="Stop after this step" aria-label="Stop" hidden><svg class="icon" aria-hidden="true"><use href="#i-stop"></use></svg></button>' +
      '<button type="button" class="composer-tool" id="chat-mic" data-action="chat-dictate" title="Dictate with Whisper" aria-label="Dictate with Whisper"><svg class="icon" aria-hidden="true"><use href="#i-mic"></use></svg></button>' +
      '<button type="button" class="composer-send" id="chat-send" data-action="chat-send" title="Send" aria-label="Send"><svg class="icon" aria-hidden="true"><use href="#i-arrow-up"></use></svg></button>' +
      "</span>" +
      "</div></div></div>" +
      '<div class="composer-status">' +
      '<span class="status-chip">devd</span>' +
      '<span class="status-sep" aria-hidden="true">/</span>' +
      '<span class="status-chip">Local</span>' +
      '<span class="status-sep" aria-hidden="true">/</span>' +
      (AI.thread ? '<span class="status-chip">' + esc(dirName(AI.thread.project || "no project")) + "</span>" : '<span class="status-chip">New conversation</span>') +
      "</div>";

    el("main").innerHTML =
      '<div class="workspace">' +
      '<div class="workspace-head">' + pageTitle("Assistant") + '<span class="grow"></span><button type="button" class="btn btn-mini" data-action="nav" data-view="agents">Team room</button></div>' +
      '<div class="assistant"><aside class="threads"><div class="threads-head"><b>Conversations</b><span class="grow"></span><button type="button" class="btn btn-mini btn-ghost" data-action="chats-prune" title="Delete every conversation that is not working">Clear finished</button><button type="button" class="btn btn-mini" data-action="chat-new">New</button></div><div id="thread-list"></div></aside>' +
      '<section class="chat">' +
      (AI.thread ? '<div class="chat-head" id="chat-head"></div>' : "") +
      (!ready ? '<div class="notice notice-warn chat-notice">No usable AI provider yet. <button type="button" class="btn btn-mini btn-primary" data-action="nav" data-view="providers">Set one up</button></div>' : "") +
      '<div class="chat-plan" id="chat-plan" hidden></div>' +
      // The conversation and the box share one column, and the box sits at the
      // bottom of it: the conversation scrolls above, the box never moves.
      '<div class="chat-body">' +
      '<div class="chat-stream" id="chat-stream" role="log" aria-live="off" aria-label="Conversation"></div>' +
      composer +
      "</div>" +
      "</section></div></div>";
    paintThreads(); paintChat(true);
    paintChatPlan();
    paintAttachments();
    fitComposer();
    var box = el("chat-stream"); if (box) box.scrollTop = box.scrollHeight;
    startThreadTimer();
  }


  // -------------------------------------------------------------------- team

  var TEAM = { runs: null, sel: null, data: null, timer: null, form: { goal: "", done: "", project: "", name: "", parallel: 2 }, say: "", open: {} };
  var TASK_WORD = { pending: "waiting", ready: "ready", running: "working", waiting_approval: "needs your approval", done: "done", unverified: "not accepted", failed: "failed", cancelled: "cancelled", skipped: "accepted by hand", blocked: "blocked" };
  var RUN_WORD = { planning: "planning", awaiting_plan: "plan needs your approval", running: "working", paused: "paused", done: "done", failed: "failed", cancelled: "cancelled" };

  function loadTeam() {
    return Promise.all([read("team.list"), state.projects.length ? Promise.resolve() : loadProjects(), loadProvidersIfNeeded(), read("template.list").catch(function () { return []; })]).then(function (r) {
      TEAM.runs = r[0] || [];
      TEAM.templates = Array.isArray(r[3]) ? r[3] : [];
      if (!TEAM.sel && TEAM.runs.length && !String(TEAM.form.goal || "").trim()) TEAM.sel = TEAM.runs[0].id;
      return TEAM.sel ? read("team.get", { run: TEAM.sel }).then(function (d) { TEAM.data = d; }).catch(function () { TEAM.data = null; }) : null;
    });
  }

  function startTeamTimer() {
    clearInterval(TEAM.timer);
    TEAM.timer = setInterval(function () {
      if (state.view !== "team") return clearInterval(TEAM.timer);
      read("team.list").then(function (runs) {
        TEAM.runs = runs;
        rail();
        return TEAM.sel ? read("team.get", { run: TEAM.sel }) : null;
      }).then(function (d) { if (d) TEAM.data = d; paintTeam(); }).catch(function () {});
    }, 1200);
  }

  function renderTeam() {
    if (state.view !== "team") return;
    if (!TEAM.runs) return renderLoading();
    var ready = (AI.providers || []).some(function (p) { return p.enabled && (p.has_key || !p.needs_key); });
    var projOpts = '<option value="">Choose a project…</option>' + state.projects.map(function (p) { return '<option value="' + attr(p.path) + '"' + (TEAM.form.project === p.path ? " selected" : "") + ">" + esc(p.name || dirName(p.path)) + "</option>"; }).join("") + '<option value="__new"' + (TEAM.form.project === "__new" ? " selected" : "") + ">A new project…</option>";
    el("main").innerHTML =
      '<div class="workspace">' +
      '<div class="workspace-head">' + pageTitle("Agent runs") + '<span class="grow"></span><button type="button" class="btn btn-mini" data-action="nav" data-view="agents">Team room</button></div>' +
      '<div class="assistant"><aside class="threads"><div class="threads-head"><b>Runs</b><span class="grow"></span><button type="button" class="btn btn-mini btn-ghost" data-action="team-prune" title="Delete every run that is over">Clear finished</button><button type="button" class="btn btn-mini" data-action="team-new">New</button></div><div id="team-list"></div></aside>' +
      '<section class="chat"><div id="team-detail" class="team-detail"></div></section></div></div>';
    TEAM.ready = ready; TEAM.projOpts = projOpts;
    paintTeam();
    startTeamTimer();
  }

  function paintTeam() {
    var list = el("team-list"), box = el("team-detail");
    if (!list || !box) return;
    list.innerHTML = (TEAM.runs || []).map(function (r) {
      var over = ["done", "failed", "cancelled"].indexOf(r.state) >= 0;
      return '<div class="thread-row"><button type="button" class="thread' + (TEAM.sel === r.id ? " active" : "") + '" data-action="team-open" data-id="' + attr(r.id) + '"><span class="dot ' + (r.state === "running" || r.state === "planning" ? "dot-ok" : r.state === "awaiting_plan" || r.state === "paused" ? "dot-warn" : r.state === "failed" ? "dot-bad" : "dot-off") + '"></span><span class="grow nowrap">' + esc(r.title) + '</span><span class="dim">' + r.tasks_done + "/" + r.tasks_total + '</span></button>' + (over ? '<button type="button" class="thread-del" data-action="team-delete" data-id="' + attr(r.id) + '" title="Delete this run" aria-label="Delete this run">\u00d7</button>' : "") + "</div>";
    }).join("") || '<div class="empty-inline">No runs yet. Give the team a goal and it will show up here.</div>';
    if (!TEAM.sel || !TEAM.data) { box.innerHTML = teamForm(); return; }
    var focus = document.activeElement && document.activeElement.id;
    var val = focus === "team-say" ? document.activeElement.value : null;
    box.innerHTML = teamDetail(TEAM.data);
    if (val !== null) { var i = el("team-say"); if (i) { i.value = val; i.focus(); var ts = el("team-say-send"); if (ts) ts.disabled = !val.trim(); } }
  }

  function teamForm() {
    var f = TEAM.form;
    return '<div class="team-new"><div class="chat-title">What should the team build?</div>' +
      '<div class="team-north-line">The team works from the project\'s north star, owner and shared context. <button type="button" data-action="nav" data-view="agents">Team room</button></div>' +
      '<div class="row-sub">A planner turns this into tasks. You review the plan and approve it before anything runs. Specialists then work in parallel where it is safe, and a task only counts as done when devd can see the proof.</div>' +
      (TEAM.ready ? "" : '<div class="notice notice-warn" style="margin-top:12px">No usable AI provider yet. <button type="button" class="btn btn-mini btn-primary" data-action="nav" data-view="providers">Set one up</button></div>') +
      (TEAM.templates && TEAM.templates.length ? '<div class="field" style="margin-top:14px"><label class="field-label" for="team-template">Start from a template <span class="dim">(optional)</span></label><select class="input" id="team-template"><option value="">Write the goal myself</option>' + TEAM.templates.map(function (t) { return '<option value="' + attr(t.id) + '"' + (TEAM.form.template === t.id ? " selected" : "") + ">" + esc(t.title) + " — " + esc(t.summary) + "</option>"; }).join("") + "</select></div>" : "") +
      '<div class="field" style="margin-top:14px"><label class="field-label" for="team-goal">Goal</label><textarea class="input" id="team-goal" rows="5" placeholder="e.g. Add password reset by email to the app, with tests. Or: Build a small Laravel blog with posts and comments.">' + esc(f.goal) + "</textarea></div>" +
      '<div class="field" style="margin-top:10px"><label class="field-label" for="team-done">Definition of done <span class="dim">(optional, one per line)</span></label><textarea class="input" id="team-done" rows="4" placeholder="e.g. The tests pass&#10;The README documents every command">' + esc(f.done) + "</textarea><div class=\"row-sub\">When the work is finished devd checks each line against what the team recorded and starts another pass for any that are not met. Needs the Jev key (judge.configure); without it the lines are listed for you to confirm.</div></div>" +
      '<div class="inline" style="margin-top:10px"><select class="input" id="team-project" aria-label="Project" style="max-width:280px">' + TEAM.projOpts + "</select>" +
      (f.project === "__new" ? '<input class="input mono" id="team-name" placeholder="new-project-name" value="' + attr(f.name) + '" style="max-width:220px" spellcheck="false">' : "") +
      '<label class="field-label" for="team-parallel">At most</label><select class="input" id="team-parallel" style="max-width:110px">' + [1, 2, 3, 4].map(function (n) { return '<option value="' + n + '"' + (f.parallel === n ? " selected" : "") + ">" + n + " at once</option>"; }).join("") + "</select>" +
      '<span class="grow"></span><button type="button" class="btn btn-primary" data-action="team-start"' + (TEAM.ready ? "" : " disabled") + ">Hand it to the team</button></div></div>";
  }

  function evidenceLine(t) {
    var ev = t.evidence || [];
    if (!ev.length) return "";
    var ch = ev.filter(function (f) { return f.kind === "change" && f.ok; }).length;
    var vp = ev.filter(function (f) { return f.kind === "verify" && f.ok; }).length, vf = ev.filter(function (f) { return f.kind === "verify" && !f.ok; }).length;
    var rd = ev.filter(function (f) { return f.kind === "read" && f.ok; }).length, cm = ev.filter(function (f) { return f.kind === "command"; }).length;
    return [ch ? ch + " change" + (ch === 1 ? "" : "s") : "", vp ? vp + " check" + (vp === 1 ? "" : "s") + " passed" : "", vf ? vf + " failed" : "", rd ? rd + " read" : "", cm ? cm + " command" + (cm === 1 ? "" : "s") : ""].filter(Boolean).join(" · ");
  }

  /* The run's lifecycle, read from what devd recorded: the run's state, its tasks
     and their evidence. A stage is only "done" when the record says so; a finished
     run with no passing check says that, rather than showing a green last step. */
  function runStages(r, work, events) {
    var st = r.state, over = ["done", "failed", "cancelled"].indexOf(st) >= 0;
    var approved = events.some(function (e) { return e.kind === "plan_approved"; }) || ["running", "paused", "done"].indexOf(st) >= 0;
    var ev = [];
    work.forEach(function (t) { ev = ev.concat(t.evidence || []); });
    var changes = ev.filter(function (f) { return f.kind === "change" && f.ok; }).length;
    var passed = ev.filter(function (f) { return f.kind === "verify" && f.ok; }).length;
    var failedChecks = ev.filter(function (f) { return f.kind === "verify" && !f.ok; }).length;
    var doneTasks = work.filter(function (t) { return t.state === "done" || t.state === "skipped"; }).length;
    var stopped = st === "failed" || st === "cancelled";
    function at(kind, note) { return { kind: kind, note: note }; }
    var plan = st === "planning" ? at("now", "planner is writing it") : work.length ? at("done", work.length + " task" + (work.length === 1 ? "" : "s")) : stopped ? at("stop", st) : at("todo", "");
    var approval = st === "awaiting_plan" ? at("wait", "needs you") : approved ? at("done", "approved") : stopped && !approved ? at(work.length ? "stop" : "todo", work.length ? "not approved" : "") : at("todo", "");
    var exec = st === "running" ? at("now", doneTasks + " of " + work.length + " done") : st === "paused" ? at("wait", "paused") :
      approved && over ? at(stopped ? "stop" : "done", doneTasks + " of " + work.length + " done") : at("todo", "");
    var evidence = changes || ev.length ? at(over || doneTasks === work.length ? "done" : "now", changes + " change" + (changes === 1 ? "" : "s") + " recorded") :
      approved && over ? at(stopped ? "stop" : "wait", "nothing recorded") : at("todo", "");
    var verify = failedChecks && !passed ? at("stop", failedChecks + " failed") :
      st === "done" ? (passed ? at("done", passed + " passed" + (failedChecks ? ", " + failedChecks + " failed" : "")) : at("wait", "no check ran")) :
      passed ? at(stopped ? "stop" : "now", passed + " passed so far") : stopped && approved ? at("stop", st) : at("todo", "");
    var list = [["Goal", at("done", "")], ["Plan", plan], ["Approval", approval], ["Execution", exec], ["Evidence", evidence], ["Verification", verify]];
    var words = { done: "done", now: "in progress", wait: "waiting", stop: "stopped", todo: "not started" };
    return '<ol class="stages" aria-label="Run progress">' + list.map(function (x, i) {
      var k = x[1].kind;
      var mark = k === "done" ? '<svg class="icon"><use href="#i-check"></use></svg>' : k === "stop" ? '<svg class="icon"><use href="#i-x"></use></svg>' : k === "wait" ? '<svg class="icon"><use href="#i-warn"></use></svg>' : String(i + 1);
      return '<li class="stage stage-' + k + '"' + (k === "now" || k === "wait" ? ' aria-current="step"' : "") + ' title="' + attr(x[0] + ": " + words[k] + (x[1].note ? " - " + x[1].note : "")) + '">' +
        '<span class="stage-mark" aria-hidden="true">' + mark + '</span><span class="stage-text"><span class="stage-label">' + x[0] + "</span>" +
        '<span class="sr-only">: ' + words[k] + "</span>" +
        (x[1].note ? '<span class="stage-note">' + esc(x[1].note) + "</span>" : "") + "</span></li>";
    }).join("") + "</ol>";
  }

  /* The goal as it was given, and whether anyone has approved a plan for it. The
     title is a short form of the goal; when the goal says more, it is shown whole. */
  function runGoal(r) {
    var goal = String(r.goal || "").trim();
    var more = goal && goal !== String(r.title || "").trim();
    return '<section class="run-goal" aria-label="Goal">' +
      (more ? '<div class="run-label">Goal in full</div><div class="run-goal-text">' + esc(goal) + "</div>" : "") +
      '<dl class="run-facts">' +
        '<div><dt>Plan</dt><dd>' + (r.plan_approved ? "Approved by a person" : '<span class="dim">Not approved. Nothing has run on this goal.</span>') + "</dd></div>" +
        '<div><dt>Up to</dt><dd>' + esc(r.parallel || 1) + (r.parallel === 1 ? " task at a time" : " tasks at once") + "</dd></div>" +
      "</dl></section>";
  }

  function teamDetail(d) {
    var r = d.run, tasks = d.tasks, events = d.events;
    var planner = tasks.filter(function (t) { return t.role === "planner"; })[0];
    var work = tasks.filter(function (t) { return t.role !== "planner"; });
    var actions = "";
    if (r.state === "awaiting_plan") actions += '<button type="button" class="btn btn-go" data-action="team-approve" data-id="' + attr(r.id) + '">Approve plan and start</button>';
    if (r.state === "running") actions += '<button type="button" class="btn" data-action="team-pause" data-id="' + attr(r.id) + '">Pause</button>';
    if (r.state === "paused") actions += '<button type="button" class="btn btn-primary" data-action="team-resume" data-id="' + attr(r.id) + '">Resume</button>';
    if (["planning", "awaiting_plan", "running", "paused"].indexOf(r.state) >= 0) actions += '<button type="button" class="btn btn-danger" data-action="team-cancel" data-id="' + attr(r.id) + '">Cancel</button>';
    else actions += '<button type="button" class="btn btn-ghost" data-action="team-delete" data-id="' + attr(r.id) + '">Delete</button>';
    if (d.run_id) actions += '<button type="button" class="btn btn-ghost" data-action="open-run" data-id="' + attr(d.run_id) + '" title="What devd recorded for this run: steps, capability calls, evidence and the durable timeline">Evidence</button>';
    var head = '<div class="chat-head"><div class="grow"><div class="chat-title">' + esc(r.title) + '</div><div class="dim mono nowrap">' + esc(r.project || "") + ' · started by ' + esc((r.created_by || "").replace("human:", "")) + " " + esc(ago(r.created_at)) + '</div></div><span class="pill ' + (r.state === "failed" ? "pill-bad" : r.state === "done" ? "pill-ok" : r.state === "awaiting_plan" || r.state === "paused" ? "pill-warn" : "pill-accent") + '">' + esc(RUN_WORD[r.state] || r.state) + "</span>" + actions + "</div>";
    var note = r.error ? '<div class="notice notice-warn" style="margin:12px 16px"><b>Needs you.</b> ' + esc(r.error) + "</div>" : "";
    if (r.state === "planning") note += '<div class="notice" style="margin:12px 16px">The planner is reading the project and writing the plan' + (planner && planner.state === "waiting_approval" ? ", and is waiting for you to approve something" : "") + ".</div>";
    var cards = work.map(function (t) {
      var open = TEAM.open[t.id], dep = t.depends_on.length ? "after " + t.depends_on.join(", ") : "starts first";
      var btns = "";
      if (["unverified", "blocked", "failed"].indexOf(t.state) >= 0) btns = '<button type="button" class="btn btn-mini btn-primary" data-action="team-retry" data-id="' + attr(t.id) + '">Retry</button><button type="button" class="btn btn-mini" data-action="team-accept" data-id="' + attr(t.id) + '" title="Accept it without the proof devd asked for. This is recorded.">Accept as is</button>';
      if (t.thread_id) btns += '<button type="button" class="btn btn-mini btn-ghost" data-action="open-thread" data-id="' + attr(t.thread_id) + '">Conversation</button>';
      return '<div class="tcard tstate-' + t.state + '"><div class="tcard-head"><span class="step-dot ' + (t.state === "done" || t.state === "skipped" ? "ok" : t.state === "running" ? "run" : t.state === "waiting_approval" ? "wait" : ["failed", "unverified", "blocked"].indexOf(t.state) >= 0 ? "bad" : "") + '"></span><b>' + esc(t.title) + '</b><span class="pill">' + esc(t.role) + '</span><span class="pill ' + (t.state === "done" ? "pill-ok" : ["failed", "unverified", "blocked"].indexOf(t.state) >= 0 ? "pill-bad" : t.state === "running" ? "pill-accent" : "") + '">' + esc(TASK_WORD[t.state] || t.state) + '</span><span class="grow"></span><button type="button" class="btn btn-mini btn-ghost" data-action="team-toggle" data-id="' + attr(t.id) + '">' + (open ? "Hide" : "Details") + "</button></div>" +
        '<div class="row-sub">' + esc(dep) + (t.attempts > 1 ? " · attempt " + t.attempts : "") + (t.branch ? " · own copy on " + esc(t.branch) : "") + (evidenceLine(t) ? " · " + esc(evidenceLine(t)) : "") + "</div>" +
        (t.error && ["unverified", "blocked", "failed"].indexOf(t.state) >= 0 ? '<div class="row-sub warn-text">' + esc(t.error) + "</div>" : "") +
        (open ? '<div class="field-label" style="margin-top:8px">Instructions</div><div class="ask-body">' + md(t.instructions) + (t.result ? '<div class="field-label" style="margin-top:8px">What it reported</div>' + md(t.result) : "") + '<div class="field-label" style="margin-top:8px">Evidence devd recorded</div>' + ((t.evidence || []).length ? '<table class="table"><tbody>' + t.evidence.map(function (f) { return "<tr><td><span class=\"pill " + (f.ok ? "pill-ok" : "pill-bad") + '">' + esc(f.kind) + '</span></td><td class="cell-mono">' + esc(f.capability) + '</td><td class="cell-mono nowrap">' + esc(f.summary || "") + "</td></tr>"; }).join("") + "</tbody></table>" : '<div class="empty-inline">None yet.</div>') + "</div>" : "") +
        (btns ? '<div class="inline" style="margin-top:8px">' + btns + "</div>" : "") + "</div>";
    }).join("") || (r.state === "planning" ? "" : '<div class="empty-inline">No tasks yet.</div>');
    var approvals = (d.approvals || []).map(function (a) { return approvalCard(a, false); }).join("");
    var log = events.filter(function (e) { return ["started", "plan", "plan_approved", "report", "accepted", "rejected", "unverified", "blocked", "merge_blocked", "failed", "retrying", "retry", "direction", "paused", "resumed", "cancelled", "accepted_by_hand", "needs_person", "done", "note", "recovered", "waiting_approval"].indexOf(e.kind) >= 0; }).slice(-60).reverse().map(function (e) {
      var x = e.data || {}, text = "";
      switch (e.kind) {
        case "started": text = x.task ? "started " + x.task + " (" + x.role + ")" : "run started"; break;
        case "plan": text = "plan: " + (x.tasks || []).length + " task(s)"; break;
        case "report": text = x.task + " reported " + x.status + ": " + String(x.summary || "").split("\n")[0]; break;
        case "accepted": text = "accepted " + x.task + (x.verified ? " (" + x.verified + " passing check" + (x.verified === 1 ? "" : "s") + ")" : ""); break;
        case "rejected": case "unverified": text = "not accepted " + x.task + ": " + x.why; break;
        case "direction": text = "you said: " + x.text; break;
        case "note": text = x.text; break;
        default: text = e.kind.replace(/_/g, " ") + (x.task ? " " + x.task : "") + (x.why ? ": " + x.why : "");
      }
      return '<div class="feed"><span class="dim mono">' + esc(ago(e.at)) + '</span><span class="grow">' + esc(text) + "</span></div>";
    }).join("");
    var summary = r.summary ? '<div class="team-result"><div class="ui-row-title">Result</div><div class="ui-row-sub wrap">' + esc(uiSummary(r.summary, 260)) + '</div><button type="button" class="btn btn-mini" data-action="team-report" data-id="' + attr(r.id) + '">Read the full report</button></div>' : "";
    var say = ["done", "failed", "cancelled"].indexOf(r.state) < 0
      ? '<div class="composer composer-row">' +
          '<textarea class="composer-input" id="team-say" rows="1" aria-label="Tell the team something new" placeholder="Add a requirement or a correction. The team picks it up on its next step."></textarea>' +
          '<button type="button" class="composer-send" data-action="team-say" data-id="' + attr(r.id) + '" id="team-say-send" disabled aria-label="Send to the team" title="Send to the team (Enter)"><svg class="icon" aria-hidden="true"><use href="#i-arrow-up"></use></svg></button>' +
        "</div>"
      : "";
    return head + '<div class="team-scroll">' + runGoal(r) + '<div class="run-track">' + runStages(r, work, events) + "</div>" + note + approvals + (summary) + '<div class="section-title" style="margin:16px 16px 8px">Tasks <span class="count">' + work.length + "</span></div>" + '<div class="tlist">' + cards + "</div>" + '<div class="section-title" style="margin:16px 16px 8px">Log</div><div class="run-log">' + (log || '<div class="empty-inline">Nothing yet.</div>') + "</div></div>" + say;
  }

  // ------------------------------------------------------------------- inbox

  function loadInbox() {
    return Promise.all([read("approval.list", { pending_only: true }), read("inbox.list", { open_only: true }), read("approval.list", { pending_only: false }), read("inbox.list", { open_only: false })]).then(function (r) {
      AI.requests = r[0]; AI.inbox = r[1];
      AI.history = { requests: (r[2] || []).filter(function (x) { return x.state !== "pending"; }).slice(0, 15), messages: (r[3] || []).filter(function (x) { return x.state === "resolved"; }).slice(0, 15) };
      return loadHealth();
    });
  }

  function renderInbox() {
    if (state.view !== "inbox") return;
    if (!AI.requests) return renderLoading();
    // Each kind of intervention is a different decision, so each has its own shape.
    var needs = [];
    AI.requests.forEach(function (r) {
      var title = (r.plan && r.plan.summary) || ("Run " + r.capability);
      var steps = (r.plan && r.plan.steps || []).map(function (x) { return "<li>" + esc(x) + "</li>"; }).join("");
      var effects = (r.plan && r.plan.effects || []).map(function (x) { return '<li class="decision-effect">' + esc(x) + "</li>"; }).join("");
      needs.push('<article class="decision" data-kind="approval">' +
        '<div class="decision-kind">Approval</div>' +
        '<div class="decision-main">' +
          '<h3 class="decision-title">' + esc(title) + "</h3>" +
          '<div class="decision-meta"><span>' + esc(r.actor.replace("agent:", "agent ")) + ' asks to run</span> <span class="mono">' + esc(r.capability) + "</span> <span>" + esc(ago(r.created_at)) + "</span></div>" +
          (steps || effects ? '<div class="decision-label">If you approve, devd will</div><ul class="decision-steps">' + steps + effects + "</ul>" : '<div class="decision-note">The request carries no description of its steps. Open the details before deciding.</div>') +
          '<details class="decision-args"><summary>Exact arguments</summary><pre class="log-block">' + esc(JSON.stringify(r.args, null, 2)) + "</pre></details>" +
          '<div class="decision-actions">' +
            '<button type="button" class="btn btn-primary" data-action="req-decide" data-id="' + attr(r.id) + '" data-approve="1">Approve and run</button>' +
            '<button type="button" class="btn" data-action="req-decide" data-id="' + attr(r.id) + '" data-approve="0">Decline</button>' +
            '<span class="decision-hint">Nothing runs until you approve. Declining tells the agent no.</span>' +
          "</div>" +
        "</div></article>");
    });
    var updates = [];
    AI.inbox.forEach(function (m) {
      var who = m.from_actor.replace("agent:", "").replace("devd-", "");
      if (m.needs_reply) {
        needs.push('<article class="decision" data-kind="question">' +
          '<div class="decision-kind">Question</div>' +
          '<div class="decision-main">' +
            '<h3 class="decision-title">' + esc(m.subject) + "</h3>" +
            '<div class="decision-meta"><span>' + esc(who) + "</span> <span>" + esc(ago(m.created_at)) + "</span></div>" +
            '<div class="decision-body">' + uiBody(m.body) + "</div>" +
            '<div class="decision-actions"><input class="input grow" id="reply-' + attr(m.id) + '" aria-label="Your answer to: ' + attr(m.subject) + '" placeholder="Your answer">' +
            '<button type="button" class="btn btn-primary" data-action="inbox-reply" data-id="' + attr(m.id) + '">Send answer</button></div>' +
          "</div></article>");
      } else {
        updates.push(uiRow({
          dot: "ok", title: esc(m.subject), sub: esc(uiSummary(m.body)),
          meta: esc(who + " · " + ago(m.created_at)),
          open: 'data-action="open-message" data-id="' + attr(m.id) + '"',
          actions: '<button type="button" class="btn btn-mini btn-ghost" data-action="inbox-reply" data-id="' + attr(m.id) + '" data-resolve="1">Dismiss</button>',
          hover: true
        }));
      }
    });
    var earlier = AI.history.requests.map(function (r) {
      return uiRow({ dot: r.state === "approved" ? "ok" : r.state === "failed" ? "bad" : "off", title: esc(r.capability), sub: esc(r.state), meta: esc(ago(r.decided_at || r.created_at)), mono: true });
    }).concat(AI.history.messages.map(function (x) {
      return uiRow({ dot: "off", title: esc(x.subject), sub: esc(x.reply ? "You answered: " + x.reply : "Read") });
    }));
    var body = "";
    if (needs.length) body += uiHead("Waiting for you", needs.length) + '<div class="decisions">' + needs.join("") + "</div>";
    if (updates.length) body += uiHead("Updates", updates.length) + uiList(updates);
    if (!needs.length && !updates.length) body += uiEmpty("You're all caught up", "When an agent has a question, or needs your okay before it changes something, it'll wait for you here. Nothing it asks for runs until you say so.");
    if (earlier.length) body += uiHead("Earlier", null, '<button type="button" class="btn btn-mini btn-ghost" data-action="inbox-clear" title="Delete every answered or dismissed message">Clear</button>') + uiList(earlier.slice(0, 8));
    el("main").innerHTML = page("Needs you", "", "", body);
  }

  function openMessageSheet(id) {
    var m = AI.inbox.filter(function (x) { return x.id === id; })[0];
    if (!m) return;
    var who = m.from_actor.replace("agent:", "");
    uiSheet(m.subject,
      '<div class="ui-quiet">' + esc(who) + " · " + esc(ago(m.created_at)) + "</div>" +
      "<div>" + uiBody(m.body) + "</div>" +
      (m.needs_reply
        ? '<div class="inline"><input class="input grow" id="reply-' + attr(m.id) + '" placeholder="Your answer"><button type="button" class="btn btn-primary" data-action="inbox-reply" data-id="' + attr(m.id) + '">Send</button></div>'
        : '<div class="ui-actions"><button type="button" class="btn" data-action="inbox-reply" data-id="' + attr(m.id) + '" data-resolve="1">Mark as read</button></div>'));
  }

  function openApprovalSheet(id) {
    var r = AI.requests.filter(function (x) { return x.id === id; })[0];
    if (!r) return;
    var steps = (r.plan && r.plan.steps || []).map(function (x) { return "<li>" + esc(x) + "</li>"; }).join("");
    var effects = (r.plan && r.plan.effects || []).map(function (x) { return '<li class="warn-text">' + esc(x) + "</li>"; }).join("");
    uiSheet((r.plan && r.plan.summary) || r.capability,
      '<div class="ui-quiet">' + esc(r.actor.replace("agent:", "")) + " · " + esc(ago(r.created_at)) + " · <span class=\"mono\">" + esc(r.capability) + "</span></div>" +
      (steps || effects ? "<h3>What will happen</h3><ul class=\"ask-steps\">" + steps + effects + "</ul>" : "") +
      '<h3>Arguments</h3><pre class="log-block">' + esc(JSON.stringify(r.args, null, 2)) + "</pre>" +
      '<div class="ui-actions"><button type="button" class="btn btn-go" data-action="req-decide" data-id="' + attr(r.id) + '" data-approve="1">Approve and run</button>' +
      '<button type="button" class="btn btn-danger" data-action="req-decide" data-id="' + attr(r.id) + '" data-approve="0">Decline</button></div>');
  }

  // ------------------------------------------------------------------- MCP
  //
  // Other programs' Model Context Protocol servers. devd is the client here:
  // their tools are offered to the assistant as mcp__<server>__<tool>, and the
  // trust column decides whether an agent's call waits for a person.

  function loadMcp() {
    return read("mcp.list").then(function (d) {
      MCP.servers = (d && d.servers) || [];
      MCP.error = "";
    }).catch(function (e) {
      MCP.servers = MCP.servers || [];
      MCP.error = e.message;
    });
  }

  function mcpWhat(s) {
    if (s.transport === "http") return s.url || "(no address)";
    return [s.command || "(no command)"].concat(s.args || []).join(" ");
  }

  function mcpTrustSelect(s) {
    var opts = [["ask", "Ask me each time"], ["allow", "Let agents use it without asking"], ["deny", "Agents may not use it"]];
    return '<select class="select compact" data-change="mcp-trust" data-name="' + attr(s.name) + '" aria-label="What agents may do with ' + attr(s.name) + '">' +
      opts.map(function (o) { return '<option value="' + o[0] + '"' + (s.trust === o[0] ? " selected" : "") + ">" + esc(o[1]) + "</option>"; }).join("") + "</select>";
  }

  function renderMcp() {
    if (state.view !== "mcp") return;
    if (!MCP.servers) return renderLoading();
    var rows = MCP.servers.map(function (s) {
      var env = (s.env_keys || []).length ? " · env: " + (s.env_keys || []).join(", ") : "";
      var tools = s.tool_count ? s.tool_count + (s.tool_count === 1 ? " tool" : " tools") : "no tools discovered yet";
      var sub = esc(mcpWhat(s)) + " · " + esc(tools) + esc(env) +
        (s.last_error ? '<div class="warn-text">' + esc(s.last_error) + "</div>" : "");
      return uiRow({
        dot: s.enabled ? (s.last_error ? "warn" : "ok") : "off",
        title: esc(s.name) + ' <span class="dim">' + esc(s.transport) + "</span>",
        sub: sub,
        actions: mcpTrustSelect(s) +
          '<button type="button" class="btn btn-mini" data-action="mcp-edit" data-name="' + attr(s.name) + '">Edit</button>' +
          '<button type="button" class="btn btn-mini" data-action="mcp-test" data-name="' + attr(s.name) + '">Test</button>' +
          '<button type="button" class="btn btn-mini btn-ghost" data-action="mcp-enable" data-name="' + attr(s.name) + '" data-on="' + (s.enabled ? "1" : "0") + '">' + (s.enabled ? "Turn off" : "Turn on") + "</button>" +
          '<button type="button" class="btn btn-mini btn-ghost" data-action="mcp-remove" data-name="' + attr(s.name) + '">Remove</button>'
      });
    }).join("");
    var toolsLine = MCP.servers.length
      ? uiHead("Servers", MCP.servers.length)
      : "";
    el("main").innerHTML = page("MCP servers", "tools from other programs, for the assistant", "",
      (MCP.error ? '<div class="notice notice-bad">' + esc(MCP.error) + "</div>" : "") +
      '<p class="dim">These are Model Context Protocol servers another program provides. The assistant sees each tool as <span class="mono">mcp__&lt;server&gt;__&lt;tool&gt;</span> and gets its result as data, never as an instruction. <b>Ask me each time</b> is the default; <b>Let agents use it without asking</b> means a model can use whatever that server can do with your permissions.</p>' +
      toolsLine + (rows ? uiList(rows) : uiEmpty("No MCP servers yet", "Add a program that speaks MCP, or import the servers Claude Code or Claude Desktop already has.", "")) +
      '<div class="panel"><div class="panel-head"><span>Add a server</span><span class="panel-note">names are lower-case; they become the tool prefix</span></div><div class="panel-body">' +
      '<div class="form-grid">' +
      '<div class="field"><label class="field-label">Name</label><input class="input" id="mcp-name" placeholder="files" spellcheck="false" autocomplete="off"></div>' +
      '<div class="field"><label class="field-label">Timeout <span class="dim">(seconds, 5-600)</span></label><input class="input" id="mcp-timeout" placeholder="120" inputmode="numeric"></div>' +
      '<div class="field field-wide"><label class="field-label">Program <span class="dim">(stdio server)</span></label><input class="input mono" id="mcp-command" placeholder="npx" spellcheck="false" autocomplete="off"></div>' +
      '<div class="field field-wide"><label class="field-label">Arguments <span class="dim">(one per line; never passed through a shell)</span></label><textarea class="input mono" id="mcp-args" rows="5" placeholder="-y&#10;@modelcontextprotocol/server-filesystem&#10;D:\\devd\\www" spellcheck="false"></textarea></div>' +
      '<div class="field field-wide"><label class="field-label">Address <span class="dim">(http server instead of a program)</span></label><input class="input mono" id="mcp-url" placeholder="https://example.test/mcp" spellcheck="false" autocomplete="off"></div>' +
      '<div class="field"><label class="field-label">Environment <span class="dim">(KEY=value, one per line; kept in a private file)</span></label><textarea class="input mono" id="mcp-env" rows="4" spellcheck="false"></textarea></div>' +
      '<div class="field"><label class="field-label">Working directory <span class="dim">(optional)</span></label><input class="input mono" id="mcp-cwd" placeholder="D:\\devd\\www" spellcheck="false"></div>' +
      "</div>" +
      '<div class="ui-actions" style="margin-top:var(--s3)"><button type="button" class="btn btn-primary" data-action="mcp-add">Add the server</button>' +
      '<button type="button" class="btn" data-action="mcp-import" title="Read ~/.claude.json, Claude Desktop, Cursor and a project .mcp.json">Import from another program</button></div>' +
      "</div></div>");
  }

  // ------------------------------------------------------------------- refresh

  function refresh() {
    state.tabs = state.tabs || {};
    if (SERVER_VIEWS.indexOf(state.view) !== -1) {
      // The toolchain probes the machine instead of reading the server
      // inventory, but it is a tab of the same screen and behaves like one.
      if (state.view === "toolchain") return loadToolchain().then(render);
      return loadServer().then(render);
    }
    if (state.view === "projects") return loadProjects().then(render);
    if (state.view === "project") {
      state.tabs = {};
      // What serves a project and which runs name it come from the server inventory and the
      // agent roster; they fill in when they arrive, they do not hold the repository back.
      if (!state.server.loaded) loadServer().then(function () { if (state.view === "project") render(); });
      if (!state.projectsLoaded) loadProjects(true).then(function () { if (state.view === "project") render(); });
      return loadProject()
        .then(function () { return loadTab(state.tab); })
        .then(render);
    }
    if (state.view === "home") return loadHome().then(render);
    if (state.view === "assistant") return loadAssistant().then(render);
    if (state.view === "team") return loadTeam().then(render);
    if (state.view === "inbox") return loadInbox().then(render);
    if (state.view === "providers") return loadProviders().then(render);
    if (state.view === "create") return loadCreate().then(render);
    if (state.view === "develop") return loadDevelop().then(render);
    if (state.view === "agents") return loadRoster().then(render);
    if (state.view === "skills") return loadSkills().then(render);
    if (state.view === "mcp") return loadMcp().then(render);
    // The project picker lists every known project, so they are read first when this is the first screen.
    if (state.view === "processes") return (state.projectsLoaded ? Promise.resolve() : loadProjects(true)).then(loadProcesses).then(render);
    if (state.view === "schedules") return loadSchedules().then(render);
    if (state.view === "shared") return loadShared().then(render);
    if (state.view === "capabilities") return loadCapabilities().then(render);
    if (state.view === "activity") return loadActivity().then(render);
    if (state.view === "system") return loadSystem().then(render);
    return Promise.resolve();
  }

  // ------------------------------------------------------------------ routes

  /* Every screen has an address.
   *
   * A link to a screen can be copied and sent to somebody, and the back
   * button means something. The path is the truth about where the person is;
   * the view is what the path resolves to. */
  var ROUTES = {
    home: "/",
    inbox: "/inbox",
    assistant: "/assistant",
    team: "/team",
    projects: "/projects",
    create: "/create",
    develop: "/develop",
    agents: "/agents",
    skills: "/skills",
    schedules: "/schedules",
    shared: "/shared",
    mcp: "/agents/mcp",
    projects: "/projects",
    processes: "/processes",
    overview: "/server",
    services: "/server/services",
    sites: "/server/sites",
    operations: "/server/operations",
    toolchain: "/server/tools",
    runtimes: "/server/runtimes",
    databases: "/server/databases",
    remote: "/server/remote",
    preview: "/server/preview",
    certificates: "/server/certificates",
    hosts: "/server/hosts",
    providers: "/models",
    activity: "/activity",
    system: "/diagnostics",
    capabilities: "/api-tools"
  };

  /* The address of the screen that is showing. */
  function routeFor(view) {
    if (view === "project") {
      var path = state.project && state.project.path;
      return path ? "/projects/" + encodeURIComponent(path) : "/projects";
    }
    if (view === "assistant") {
      return AI.thread && AI.thread.id ? "/assistant/" + encodeURIComponent(AI.thread.id) : "/assistant";
    }
    if (view === "site") {
      return SITE.name ? "/server/sites/" + encodeURIComponent(SITE.name) : "/server/sites";
    }
    return ROUTES[view] || "/";
  }

  /* The screen an address means. */
  function viewFor(path) {
    var p = String(path || "/").split("?")[0].replace(/\/+$/, "") || "/";
    if (p === "/" ) return "home";
    if (p.indexOf("/projects/") === 0) return "project";
    if (p.indexOf("/assistant/") === 0) return "assistant";
    if (p.indexOf("/server/sites/") === 0) return "site";
    for (var k in ROUTES) {
      if (ROUTES[k] === p) return k;
    }
    return "home";
  }

  /* Put the address in the bar without adding a history entry. */
  function writeRoute(view, replace) {
    var path = routeFor(view);
    if (location.pathname + location.search === path) return;
    var url = path + location.search;
    if (replace) history.replaceState({ view: view }, "", url);
    else history.pushState({ view: view }, "", url);
  }

  /* Follow the address: the back button, a pasted link, a bookmark. */
  function readRoute() {
    var view = viewFor(location.pathname);
    var parts = location.pathname.split("/").filter(Boolean);
    var site = "";
    if (view === "site") {
      var rawSite = parts.slice(2).join("/");
      if (rawSite) {
        try { site = decodeURIComponent(rawSite); } catch (e) { site = ""; }
      }
      if (!site) view = "sites";
    }
    if (view === "project") {
      var raw = parts.slice(1).join("/");
      if (raw) {
        try { state.project = { path: decodeURIComponent(raw), name: dirName(decodeURIComponent(raw)) }; } catch (e) { /* not a path */ }
      }
    }
    if (view === "assistant" && parts[1]) {
      state.pendingThread = decodeURIComponent(parts[1]);
    }
    var siteChanged = view === "site" && site !== SITE.name;
    if (site) {
      SITE.name = site;
      if (siteChanged) {
        SITE.tab = "overview";
        OPS.data = {};
        OPS.error = "";
        OPS.errorTab = "";
      }
    }
    if (state.view === view && !state.pendingThread && !siteChanged) return;
    navigate(view, true);
  }

  function navigate(view, fromRoute) {
    if (state.topMenu) { state.topMenu = ""; }
    if (AREA[view]) { state.areaTab = state.areaTab || {}; state.areaTab[AREA[view]] = view; }
    var moved = state.view !== view;
    // A sheet belongs to the screen it was opened on; going elsewhere closes it.
    if (moved && !el("drawer").hidden) { drawerOrigin = null; closeDrawer(); }
    var ae = document.activeElement;
    var fromNav = !!(ae && ae.closest && ae.closest(".rail, #subnav"));
    state.view = view;
    if (!fromRoute) writeRoute(view, false);
    render();
    // A new screen starts at its top; re-rendering the same one keeps its place.
    var scroller = moved && document.querySelector(".content");
    if (scroller) scroller.scrollTop = 0;
    // Keyboard and screen-reader users land on the new screen, not on the page's body.
    if (moved && fromNav && el("main")) el("main").focus({ preventScroll: true });
    return refresh();
  }

  // ------------------------------------------------------------------ studio

  function studioProject() {
    if (state.view === "project" && state.project && state.project.path) return state.project.path;
    if (STUDIO.project) return STUDIO.project;
    if (state.project && state.project.path) return state.project.path;
    return state.projects[0] ? state.projects[0].path : "";
  }

  function loadStudio() {
    var ready = state.projects.length ? Promise.resolve() : read("project.list").then(function (d) {
      state.projects = Array.isArray(d) ? d : [];
    }).catch(function () { /* the empty state explains itself */ });
    return ready.then(function () {
      if (STUDIO.project === "") STUDIO.project = studioProject();
      return Promise.all([
        read("review.list", { limit: 30 }).catch(function () { return []; }),
        read("memory.list", { limit: 30 }).catch(function () { return []; })
      ]).then(function (r) {
        STUDIO.reviews = Array.isArray(r[0]) ? r[0] : [];
        STUDIO.memory = Array.isArray(r[1]) ? r[1] : [];
      });
    });
  }

  function studioProjectBar(extra) {
    var list = (state.projects || []).filter(function (p) { return p && p.path; });
    if (!list.length) {
      return '<div class="panel"><div class="panel-body empty-inline">No project yet. <button type="button" class="btn btn-mini btn-primary" data-action="nav" data-view="projects">Add one</button></div></div>';
    }
    var chosen = studioProject();
    var options = list.map(function (p) {
      return '<option value="' + attr(p.path) + '"' + (p.path === chosen ? " selected" : "") + ">" + esc(p.name || dirName(p.path)) + "</option>";
    }).join("");
    return '<div class="toolbar"><select class="select" id="studio-project" title="' + attr(chosen) + '">' + options + "</select>" +
      '<span class="grow"></span>' + (extra || "") + "</div>";
  }

  function severityPill(sev) {
    var s = String(sev || "").toLowerCase();
    return '<span class="pill ' + (s === "high" ? "pill-bad" : s === "medium" ? "pill-warn" : "") + '">' + esc(s || "note") + "</span>";
  }

  function renderReviewTab(embedded) {
    var project = studioProject();
    var list = (STUDIO.reviews || []).filter(function (r) { return !project || r.project === project; });
    var selected = STUDIO.selected;
    var rows = list.map(function (r) {
      var count = (r.findings || []).length;
      return '<button type="button" class="note-row' + (selected && selected.id === r.id ? " active" : "") + '" data-action="studio-open-review" data-id="' + attr(r.id) + '">' +
        '<span class="cell-strong">' + esc(r.summary || "(no summary)") + "</span>" +
        '<span class="cell-mono">' + (count ? count + " finding" + (count === 1 ? "" : "s") : "no findings") + " · " + esc(ago(r.created_at)) + "</span></button>";
    }).join("");
    var detail = "";
    if (selected) {
      var findings = (selected.findings || []).map(function (f) {
        return '<div class="finding"><div class="finding-head">' + severityPill(f.severity) + "<b>" + esc(f.title || "Finding") + "</b>" +
          (f.file ? '<span class="cell-mono dim">' + esc(f.file) + (f.line ? ":" + esc(f.line) : "") + "</span>" : "") + "</div>" +
          (f.detail ? '<div class="ask-body">' + md(f.detail) + "</div>" : "") +
          (f.suggestion ? '<div class="row-sub">Suggestion: ' + esc(f.suggestion) + "</div>" : "") + "</div>";
      }).join("");
      detail = '<div class="panel"><div class="panel-head"><span>' + esc(selected.model || "Review") + '</span><span class="panel-note">' + esc(ago(selected.created_at)) + "</span>" +
        '<button type="button" class="btn btn-mini btn-ghost" data-action="studio-delete-review" data-id="' + attr(selected.id) + '">Delete</button></div>' +
        '<div class="panel-body stack">' + (selected.summary ? md(selected.summary) : "") +
        (findings || '<div class="empty-inline">No findings.</div>') +
        (selected.diff ? '<details><summary class="row-sub">The diff it read' + (selected.diff_truncated ? " (shortened)" : "") + '</summary><pre class="log-block">' + esc(selected.diff) + "</pre></details>" : "") +
        "</div></div>";
    }
    return (embedded ? "" : studioProjectBar('<button type="button" class="btn btn-primary" data-action="studio-review"' + (project ? "" : " disabled") + ">Review changes</button>")) +
      '<div class="note-layout"><div class="panel note-list-panel"><div class="panel-head"><span>Reviews</span><span class="panel-note">' + list.length + "</span></div>" +
      '<div class="panel-body flush">' + (rows || '<div class="empty-inline">No reviews yet.</div>') + "</div></div>" +
      (detail || '<div class="panel"><div class="panel-body empty-inline">Choose a review, or run one.</div></div>') + "</div>";
  }

  function renderMemoryTab(embedded) {
    var project = studioProject();
    var proposals = (STUDIO.memory || []).filter(function (p) { return !project || p.project === project; });
    var rows = proposals.map(function (p) {
      return '<button type="button" class="note-row' + (STUDIO.memoryOpen && STUDIO.memoryOpen.id === p.id ? " active" : "") + '" data-action="studio-open-memory" data-id="' + attr(p.id) + '">' +
        '<span class="cell-strong">' + esc(p.lesson) + '</span><span class="cell-mono">' + esc(ago(p.created_at)) + "</span></button>";
    }).join("");
    var open = STUDIO.memoryOpen;
    var detail = open
      ? '<div class="panel"><div class="panel-head"><span>' + esc(open.target || "AGENTS.md") + "</span>" +
        '<button type="button" class="btn btn-mini btn-primary" data-action="studio-apply-memory" data-id="' + attr(open.id) + '">Apply</button></div>' +
        '<div class="panel-body stack">' + (open.evidence ? '<div class="row-sub">Evidence: ' + esc(open.evidence) + "</div>" : "") +
        '<pre class="log-block">' + esc(open.diff || "") + "</pre></div></div>"
      : '<div class="panel"><div class="panel-body empty-inline">Choose a proposal to see its diff.</div></div>';
    var c = STUDIO.compat;
    var audit = "";
    if (c) {
      var files = (c.files || []).map(function (f) {
        return "<tr><td class=\"cell-strong\">" + esc(f.path) + "</td><td>" + esc(f.owner) + "</td></tr>";
      }).join("");
      var issues = (c.issues || []).map(function (i) {
        return '<div class="finding"><div class="finding-head">' + severityPill(i.severity) + "<b>" + esc(i.path || "Project") + '</b></div><div class="ask-body">' + esc(i.message) + "</div></div>";
      }).join("");
      audit = '<div class="panel"><div class="panel-head"><span>Instruction files</span><span class="panel-note">' + (c.files || []).length + "</span>" +
        (c.proposal ? '<button type="button" class="btn btn-mini btn-primary" data-action="studio-compat-write">Create AGENTS.md</button>' : "") + "</div>" +
        '<div class="panel-body stack">' + (files ? '<table class="table"><thead><tr><th>File</th><th>Read by</th></tr></thead><tbody>' + files + "</tbody></table>" : '<div class="empty-inline">No instruction file found.</div>') +
        issues + "</div></div>";
    }
    return (embedded ? "" : studioProjectBar('<button type="button" class="btn" data-action="studio-compat"' + (studioProject() ? "" : " disabled") + ">Audit instruction files</button>")) +
      '<div class="panel"><div class="panel-head"><span>Remember a lesson</span></div><div class="panel-body stack">' +
      '<textarea class="input" id="studio-memory-lesson" rows="2" placeholder="One durable fact: a build command, a convention, a trap."></textarea>' +
      '<div class="inline"><input class="input grow" id="studio-memory-evidence" placeholder="What proved it?">' +
      '<button type="button" class="btn btn-primary" data-action="studio-propose-memory"' + (studioProject() ? "" : " disabled") + ">Propose</button></div></div></div>" +
      '<div class="note-layout"><div class="panel note-list-panel"><div class="panel-head"><span>Proposals</span><span class="panel-note">' + proposals.length + "</span></div>" +
      '<div class="panel-body flush">' + (rows || '<div class="empty-inline">None pending.</div>') + "</div></div>" + detail + "</div>" + audit;
  }

  function renderStudioTarget() {
    if (state.view !== "project") return;
    if (state.tab === "reviews") state.tabs.reviews = { loading: false, data: STUDIO.reviews || [] };
    if (state.tab === "memory") state.tabs.memory = { loading: false, data: STUDIO.memory || [] };
    render();
  }

  function renderLoading(note) {
    el("main").innerHTML =
      '<div class="page"><div class="loading" role="status" aria-live="polite">Loading…</div>' +
      (note ? '<div class="loading-note">' + esc(note) + "</div>" : "") + "</div>";
  }

  /* Reading the server means probing this machine - installed services,
     runtimes, sites and the inventory - which takes seconds on a cold start.
     The note says what is being waited for, so the screen is not a spinner
     with no explanation. */
  function renderLoadingServer() {
    renderLoading("Checking the services, runtimes and sites on this machine.");
  }

  /* Tables are the one element that cannot shrink below its content. Wrapping
     every rendered table makes a narrow window scroll the table instead of the
     page. The observer catches views that repaint a panel in place. */
  function wrapTables() {
    var main = el("main");
    if (!main) return;
    Array.prototype.forEach.call(main.querySelectorAll("table.table"), function (table) {
      if (table.parentElement && table.parentElement.classList.contains("table-wrap")) return;
      var wrap = document.createElement("div");
      wrap.className = "table-wrap";
      table.parentNode.insertBefore(wrap, table);
      wrap.appendChild(table);
    });
  }

  function render() {
    rail();
    subnav();
    crumbs();
    statusbar();
    healthChip();
    if (SERVER_VIEWS.indexOf(state.view) !== -1) {
      if (!state.server.loaded && state.view !== "toolchain") return renderLoadingServer();
      if (state.view === "overview") renderOverview();
      else if (state.view === "sites") renderSites();
      else if (state.view === "site") renderSite();
      else if (state.view === "operations") renderOperations();
      else if (state.view === "preview") renderPreview();
      else if (state.view === "services") renderServices();
      else if (state.view === "runtimes") renderRuntimes();
      else if (state.view === "databases") renderDatabases();
      else if (state.view === "remote") renderRemote();
      else if (state.view === "certificates") renderCertificates();
      else if (state.view === "hosts") renderHosts();
      else if (state.view === "toolchain") renderToolchain();
    } else if (state.view === "home") renderHome();
    else if (state.view === "assistant") renderAssistant();
    else if (state.view === "team") renderTeam();
    else if (state.view === "inbox") renderInbox();
    else if (state.view === "providers") renderProviders();
    else if (state.view === "create") renderCreate();
    else if (state.view === "develop") renderDevelop();
    else if (state.view === "agents") renderRoster();
    else if (state.view === "skills") renderSkills();
    else if (state.view === "mcp") renderMcp();
    else if (state.view === "processes") renderProcesses();
    else if (state.view === "schedules") renderSchedules();
    else if (state.view === "shared") renderShared();
    else if (state.view === "projects") renderProjects();
    else if (state.view === "project") renderProject();
    else if (state.view === "capabilities") renderCapabilities();
    else if (state.view === "activity") renderActivity();
    else if (state.view === "system") renderSystem();
    wrapTables();
  }

  // -------------------------------------------------------------------- actions

  function project() {
    return state.project ? state.project.path : null;
  }

  function readInput(id) {
    var node = el(id);
    return node ? node.value.trim() : "";
  }

  function checked(id) {
    var node = el(id);
    return node ? node.checked : false;
  }

  // ---------------------------------------------------------- the composer
  //
  // The box takes a message, files and speech. Files are uploaded and their
  // paths are remembered; speech is recorded in the browser as 16 kHz mono
  // WAV - the shape Whisper wants - and transcribed on this machine.

  var ATTACHMENTS = [];
  var RECORDER = null;

  /* The strip along the top of a composer: the paperclip, and beside it the
     files that will go with the message. It lives inside the composer, above
     the words, so what is attached is read together with what is asked. */
  function composerTop(prefix) {
    return '<div class="composer-attachments" id="' + prefix + '-attachments"' + (ATTACHMENTS.length ? "" : " hidden") + ">" + attachmentChips() + "</div>" +
      '<input type="file" id="' + prefix + '-file" multiple hidden>';
  }

  /* The paperclip sits in the bar, beside the project it works in. */
  function attachButton(prefix) {
    return '<button type="button" class="composer-attach" data-action="chat-attach" data-input="' + prefix + '-file" title="Attach files or photos" aria-label="Attach files or photos"><svg class="icon" aria-hidden="true"><use href="#i-paperclip"></use></svg></button>';
  }

  /* Who takes this: one selector, in every composer. */
  function modeSelect() {
    var team = AI.homeMode === "team";
    return '<select class="composer-picker composer-mode" id="composer-mode" aria-label="Who takes this" title="The assistant answers one request. Agent runs plan a goal and wait for your approval before anything runs.">' +
      '<option value="ask"' + (team ? "" : " selected") + ">Assistant</option>" +
      '<option value="team"' + (team ? " selected" : "") + ">Agent runs</option></select>";
  }

  function attachmentChips() {
    return ATTACHMENTS.map(function (a, i) {
      var thumb = a.preview ? '<img src="' + attr(a.preview) + '" alt="">' : '<svg class="icon" aria-hidden="true"><use href="#i-file"></use></svg>';
      return '<span class="composer-attachment' + (a.preview ? " is-image" : "") + '">' + thumb + '<b title="' + attr(a.path) + '">' + esc(a.name) + "</b>" +
        '<button type="button" data-action="chat-remove-file" data-index="' + i + '" aria-label="Remove ' + attr(a.name) + '">×</button></span>';
    }).join("");
  }

  /* Fold the attached files into what is sent, and clear them. */
  function takeAttachments(text) {
    if (ATTACHMENTS.length) {
      text += (text ? "\n\n" : "") + "Attached files:\n" + ATTACHMENTS.map(function (a) { return "- " + a.path; }).join("\n");
    }
    ATTACHMENTS = [];
    paintAttachments();
    return text;
  }

  function addFiles(files) {
    return Promise.all(files.map(function (f) {
      return uploadFile(f).then(function (stored) {
        if (/^image\//.test(f.type || "") && window.URL) stored.preview = URL.createObjectURL(f);
        return stored;
      });
    })).then(function (stored) {
      ATTACHMENTS = ATTACHMENTS.concat(stored);
      paintAttachments();
    }).catch(function (err) { toast("error", err.message); });
  }

  function paintAttachments() {
    ["chat-attachments", "home-attachments"].forEach(function (id) {
      var box = el(id);
      if (!box) return;
      box.hidden = !ATTACHMENTS.length;
      box.innerHTML = attachmentChips();
    });
    syncComposerAction();
  }

  function uploadFile(file) {
    return fetch("/api/upload", {
      method: "POST",
      headers: { Authorization: "Bearer " + TOKEN, "X-File-Name": file.name || "file" },
      body: file
    }).then(function (res) {
      if (!res.ok) {
        return res.json().then(function (body) {
          throw new Error((body && body.error && body.error.message) || "the file could not be uploaded");
        });
      }
      return res.json();
    });
  }

  // ----------------------------------------------------------------- mentions

  /* What a message can point at.
   *
   * Typing "@" opens a list of the things an agent can be given: a capability
   * it can run, a skill it can load, a project it can work in. The mention is
   * written into the message as text, so the agent reads exactly what the
   * person wrote - the list is a way of naming things, not a side channel. */
  var MENTIONS = { open: false, items: [], index: 0, start: 0, query: "", seq: 0, timer: null };

  /* Where the "@" that opens the list is, and what has been typed after it. */
  function mentionTrigger(text, caret) {
    if (caret == null) caret = text.length;
    var upto = text.slice(0, caret);
    var at = upto.lastIndexOf("@");
    if (at < 0) return null;
    // An "@" inside a word is part of that word - an address, a version - and
    // should not open a list.
    if (at > 0 && !/[\s(]/.test(upto.charAt(at - 1))) return null;
    var query = upto.slice(at + 1);
    if (/[\s]/.test(query)) return null;
    return { start: at, query: query };
  }

  function mentionItems(query) {
    var q = query.trim();
    var caps = request("/api/capabilities?limit=8&q=" + encodeURIComponent(q))
      .then(function (res) { return res.ok ? (res.body.items || []) : []; })
      .catch(function () { return []; });
    var skills = read("skills.list", { query: q })
      .then(function (d) { return Array.isArray(d) ? d : []; })
      .catch(function () { return []; });
    var projects = read("project.list")
      .then(function (d) { return Array.isArray(d) ? d : []; })
      .catch(function () { return []; });
    var roster = read("team.roles")
      .then(function (d) { return Array.isArray(d) ? d : []; })
      .catch(function () { return []; });
    return Promise.all([caps, skills, projects, roster]).then(function (r) {
      var items = [];
      r[0].forEach(function (c) {
        items.push({ kind: "capability", token: "@" + c.id, title: c.id, detail: c.summary || c.group, group: "Capabilities" });
      });
      r[1].forEach(function (s) {
        if (q && s.name.indexOf(q.toLowerCase()) < 0 && String(s.description || "").toLowerCase().indexOf(q.toLowerCase()) < 0) return;
        items.push({ kind: "skill", token: "@skill:" + s.name, title: s.name, detail: s.description, group: "Skills" });
      });
      r[2].forEach(function (p) {
        var name = p.name || dirName(p.path || "");
        if (q && name.toLowerCase().indexOf(q.toLowerCase()) < 0 && String(p.path || "").toLowerCase().indexOf(q.toLowerCase()) < 0) return;
        items.push({ kind: "project", token: "@project:" + name, title: name, detail: p.path, path: p.path, group: "Projects" });
      });
      // The members of devd's team a message can be handed to.
      (r[3] || []).forEach(function (m) {
        if (q && m.id.indexOf(q.toLowerCase()) < 0 && m.title.toLowerCase().indexOf(q.toLowerCase()) < 0 && String(m.summary || "").toLowerCase().indexOf(q.toLowerCase()) < 0) return;
        items.push({ kind: "member", token: "@" + m.id + ":", title: m.title, detail: m.summary, group: "devd's team", member: m.id });
      });
      return items;
    });
  }

  function openMentions(start, query) {
    MENTIONS.start = start;
    MENTIONS.query = query;
    MENTIONS.open = true;
    MENTIONS.index = 0;
    var seq = ++MENTIONS.seq;
    clearTimeout(MENTIONS.timer);
    // The list is a suggestion, so it appears at once and refines itself a
    // moment later rather than leaving the person waiting.
    paintMentions();
    MENTIONS.timer = setTimeout(function () {
      if (seq !== MENTIONS.seq) return;
      mentionItems(query).then(function (items) {
        if (seq !== MENTIONS.seq) return;
        MENTIONS.items = items;
        MENTIONS.index = 0;
        paintMentions();
      });
    }, 120);
  }

  function closeMentions() {
    MENTIONS.open = false;
    MENTIONS.items = [];
    MENTIONS.seq += 1;
    clearTimeout(MENTIONS.timer);
    var box = el("chat-mentions");
    if (box) { box.hidden = true; box.innerHTML = ""; }
  }

  function paintMentions() {
    var box = el("chat-mentions");
    if (!box) return;
    if (!MENTIONS.open) { box.hidden = true; box.innerHTML = ""; return; }
    if (!MENTIONS.items.length) {
      box.hidden = false;
      box.innerHTML = '<div class="mention-empty">Nothing to reference yet.</div>';
      return;
    }
    var groups = [];
    var byGroup = {};
    MENTIONS.items.forEach(function (it, i) {
      if (!byGroup[it.group]) { byGroup[it.group] = []; groups.push(it.group); }
      byGroup[it.group].push({ it: it, i: i });
    });
    box.innerHTML = groups.map(function (g) {
      return '<div class="mention-group"><div class="mention-label">' + esc(g) + "</div>" +
        byGroup[g].map(function (row) {
          var it = row.it;
          return '<button type="button" class="mention-item' + (row.i === MENTIONS.index ? " active" : "") +
            '" data-action="mention-pick" data-index="' + row.i + '" role="option" aria-selected="' + (row.i === MENTIONS.index ? "true" : "false") + '">' +
            '<span class="mention-title">' + esc(it.title) + "</span>" +
            (it.detail ? '<span class="mention-detail">' + esc(it.detail) + "</span>" : "") +
            "</button>";
        }).join("") + "</div>";
    }).join("");
    box.hidden = false;
    var active = box.querySelector(".mention-item.active");
    if (active) active.scrollIntoView({ block: "nearest" });
  }

  function moveMention(step) {
    if (!MENTIONS.items.length) return;
    var n = MENTIONS.items.length;
    MENTIONS.index = (MENTIONS.index + step + n) % n;
    paintMentions();
  }

  function pickMention() {
    var it = MENTIONS.items[MENTIONS.index];
    if (!it) return;
    applyMention(it);
  }

  /* Put the mention into the message and, for a project, say where the work
   * happens. */
  function applyMention(item) {
    var inp = el("chat-input");
    if (!inp) return;
    var text = inp.value;
    var caret = inp.selectionStart == null ? text.length : inp.selectionStart;
    var before = text.slice(0, MENTIONS.start);
    var after = text.slice(caret);
    var token = item.token + " ";
    inp.value = before + token + after;
    AI.composer = inp.value;
    var pos = before.length + token.length;
    inp.focus();
    try { inp.setSelectionRange(pos, pos); } catch (e) { /* not a text control */ }
    closeMentions();
    syncComposerAction();
    fitComposer();
    if (item.kind === "project" && item.path && !AI.thread) {
      AI.project = item.path;
      var sel = el("chat-project");
      if (sel) {
        var known = Array.prototype.some.call(sel.options, function (o) { return o.value === item.path; });
        if (!known) {
          var opt = document.createElement("option");
          opt.value = item.path;
          opt.textContent = item.title;
          sel.appendChild(opt);
        }
        sel.value = item.path;
      }
    }
  }

  /* The microphone and the send arrow share one slot. The microphone holds it
   * until there is something to send; typing takes the slot back. */
  function syncComposerAction() {
    var inp = el("chat-input"), send = el("chat-send"), mic = el("chat-mic");
    if (!inp || !send || !mic) return;
    var busy = !!(AI.thread && (AI.thread.state === "running" || AI.running));
    var hasText = !!(inp.value || "").trim() || ATTACHMENTS.length;
    // One primary action in the slot, and it says what pressing it does now:
    // while the agent works with nothing typed, it stops; the moment there is
    // text it becomes the arrow, which sends (or queues when it is still
    // working); clearing the draft brings the stop back.
    send.hidden = busy ? hasText : !hasText;
    mic.hidden = busy || hasText;
    var stop = el("chat-stop");
    if (stop) stop.hidden = !busy || hasText;
  }

  /* The box grows with what is written in it, up to a share of the window, and
   * then scrolls inside itself. It rests at its normal height again when it is
   * emptied, so a long draft never leaves the conversation cramped afterwards. */
  function fitComposer() {
    var inp = el("chat-input");
    if (!inp) return;
    var max = Math.max(120, Math.min(360, Math.round(window.innerHeight * 0.4)));
    inp.style.height = "auto";
    var natural = inp.scrollHeight;
    inp.style.height = Math.min(natural, max) + "px";
    inp.style.overflowY = natural > max ? "auto" : "hidden";
  }

  /* Record 16 kHz mono PCM and encode it as a WAV file.
   *
   * Whisper wants 16 kHz mono. Encoding it here means the server never has to
   * decode a compressed format, and the recording is exactly the shape the
   * model was trained on. */
  function recordWav(seconds) {
    return navigator.mediaDevices.getUserMedia({ audio: true }).then(function (stream) {
      var context = new (window.AudioContext || window.webkitAudioContext)();
      var source = context.createMediaStreamSource(stream);
      var processor = context.createScriptProcessor(4096, 1, 1);
      var chunks = [];
      processor.onaudioprocess = function (event) {
        chunks.push(new Float32Array(event.inputBuffer.getChannelData(0)));
      };
      source.connect(processor);
      processor.connect(context.destination);
      return new Promise(function (resolve) {
        RECORDER = {
          stop: function () {
            try { source.disconnect(); processor.disconnect(); } catch (e) { /* already gone */ }
            stream.getTracks().forEach(function (t) { t.stop(); });
            var rate = context.sampleRate;
            context.close();
            resolve({ rate: rate, chunks: chunks });
          }
        };
        setTimeout(function () {
          if (RECORDER) { var r = RECORDER; RECORDER = null; r.stop(); }
        }, seconds * 1000);
      });
    });
  }

  function encodeWav(chunks, rate) {
    var total = chunks.reduce(function (n, c) { return n + c.length; }, 0);
    var target = 16000;
    var ratio = rate / target;
    var outLength = Math.floor(total / ratio);
    var out = new Float32Array(outLength);
    for (var i = 0; i < total; i++) {
      var to = Math.floor(i / ratio);
      if (to < outLength) out[to] = chunks[0][i];
    }
    var buffer = new ArrayBuffer(44 + outLength * 2);
    var view = new DataView(buffer);
    var write = function (at, text) {
      for (var i = 0; i < text.length; i++) view.setUint8(at + i, text.charCodeAt(i));
    };
    write(0, "RIFF");
    view.setUint32(4, 36 + outLength * 2, true);
    write(8, "WAVE");
    write(12, "fmt ");
    view.setUint32(16, 16, true);
    view.setUint16(20, 1, true);
    view.setUint16(22, 1, true);
    view.setUint32(24, target, true);
    view.setUint32(28, target * 2, true);
    view.setUint16(32, 2, true);
    view.setUint16(34, 16, true);
    write(36, "data");
    view.setUint32(40, outLength * 2, true);
    for (var s = 0; s < outLength; s++) {
      var v = Math.max(-1, Math.min(1, out[s]));
      view.setInt16(44 + s * 2, v < 0 ? v * 0x8000 : v * 0x7fff, true);
    }
    return new Blob([buffer], { type: "audio/wav" });
  }

  function insertAtCursor(text) {
    var input = el("chat-input");
    if (!input) return;
    var start = input.selectionStart || input.value.length;
    var end = input.selectionEnd || start;
    input.value = input.value.slice(0, start) + text + input.value.slice(end);
    var at = start + text.length;
    input.focus();
    try { input.setSelectionRange(at, at); } catch (e) { /* not a text field */ }
  }



  var ACTIONS = {
    "chat-attach": function (node) {
      var input = el((node && node.dataset.input) || "chat-file");
      if (input) input.click();
    },

    /* A starting point is put in the box, not sent: the person can edit it,
       and nothing runs until they press send. */
    "chat-suggest": function (node) {
      var prompt = node.dataset.prompt || "";
      AI.composer = prompt;
      var input = el("chat-input");
      if (input) {
        input.value = prompt;
        input.focus();
        input.setSelectionRange(prompt.length, prompt.length);
      }
      syncComposerAction();
    },

    // --------------------------------------------------------------- reviews
    "studio-review": function () {
      var project = studioProject();
      if (!project) return toast("warn", "Choose a project first");
      toast("info", "Reviewing the changes. The diff goes to the model chosen for reviewers.");
      run("review.run", { project: project }).then(function (o) {
        if (!o) return;
        STUDIO.selected = o.data || null;
        return loadStudio().then(renderStudioTarget);
      });
    },

    "studio-open-review": function (node) {
      read("review.get", { id: node.dataset.id }).then(function (r) {
        STUDIO.selected = r;
        renderStudioTarget();
      }).catch(function (e) { toast("error", e.message); });
    },

    "studio-delete-review": function (node) {
      run("review.delete", { id: node.dataset.id }).then(function (o) {
        if (!o) return;
        STUDIO.selected = null;
        return loadStudio().then(renderStudioTarget);
      });
    },

    "studio-propose-memory": function () {
      var project = studioProject();
      if (!project) return toast("warn", "Choose a project first");
      var lesson = (el("studio-memory-lesson") || {}).value || "";
      if (!lesson.trim()) return toast("warn", "Say what should be remembered");
      run("memory.propose", { project: project, lesson: lesson.trim(), evidence: readInput("studio-memory-evidence") || null }).then(function (o) {
        if (!o) return;
        STUDIO.memoryOpen = o.data;
        return loadStudio().then(renderStudioTarget);
      });
    },

    "studio-open-memory": function (node) {
      var found = (STUDIO.memory || []).filter(function (p) { return p.id === node.dataset.id; })[0];
      STUDIO.memoryOpen = found || null;
      renderStudioTarget();
    },

    "studio-apply-memory": function (node) {
      run("memory.apply", { id: node.dataset.id }).then(function (o) {
        if (!o) return;
        STUDIO.memoryOpen = null;
        return loadStudio().then(renderStudioTarget);
      });
    },

    "studio-compat": function () {
      var project = studioProject();
      if (!project) return toast("warn", "Choose a project first");
      read("compat.audit", { project: project }).then(function (c) {
        STUDIO.compat = c;
        renderStudioTarget();
      }).catch(function (e) { toast("error", e.message); });
    },

    "studio-compat-write": function () {
      var project = studioProject();
      if (!project) return;
      run("compat.write", { project: project }).then(function (o) {
        if (!o) return;
        return read("compat.audit", { project: project }).then(function (c) {
          STUDIO.compat = c;
          renderStudioTarget();
        });
      });
    },

    "chat-remove-file": function (node) {
      ATTACHMENTS.splice(Number(node.dataset.index), 1);
      paintAttachments();
    },

    "chat-dictate": function (node) {
      if (RECORDER) {
        var recorder = RECORDER;
        RECORDER = null;
        node.classList.remove("is-recording");
        recorder.stop().then(function (audio) {
          if (!audio.chunks.length) {
            toast("warn", "Nothing was recorded");
            return;
          }
          var blob = encodeWav(audio.chunks, audio.rate);
          uploadFile(new File([blob], "dictation.wav", { type: "audio/wav" }))
            .then(function (stored) {
              return read("dictation.transcribe", { path: stored.path });
            })
            .then(function (data) {
              var text = (data && data.text) || "";
              if (!text) {
                toast("warn", "Whisper heard nothing");
                return;
              }
              insertAtCursor(text);
            })
            .catch(function (e) {
              toast("error", e.message);
            });
        });
        return;
      }
      if (!navigator.mediaDevices || !navigator.mediaDevices.getUserMedia) {
        toast("error", "This browser cannot record audio");
        return;
      }
      toast("info", "Recording. Press the microphone again to stop.");
      node.classList.add("is-recording");
      recordWav(120)
        .then(function (recorder) {
          RECORDER = recorder;
        })
        .catch(function (e) {
          node.classList.remove("is-recording");
          toast("error", "The microphone could not be opened: " + e.message);
        });
    },

    nav: function (node) {
      var v = node.dataset.view, area = AREA[v];
      if (node.classList.contains("rail-item") && !node.dataset.exact && area && AREA[state.view] !== area && state.areaTab && state.areaTab[area]) v = state.areaTab[area];
      navigate(v);
    },

    "svc-option": function (n) {
      var tool = n.dataset.tool, opt = n.dataset.opt;
      var done = function (o) { if (o) refresh(); };
      if (opt === "clear") return run("services.decide", { tool: tool, clear: true }).then(done);
      if (opt === "adopt" || opt === "observe" || opt === "ignore") return run("services.decide", { tool: tool, mode: opt }).then(done);
      if (opt === "external") {
        return uiPrompt({ title: "Your own " + tool, text: "Which port does your instance listen on?", input: { placeholder: "e.g. 5432" }, confirm: "Use this port" }).then(function (v) {
          var port = parseInt(v, 10);
          if (!port) return null;
          return run("services.decide", { tool: tool, mode: "external", port: port }).then(done);
        });
      }
      if (opt === "start") return run("toolchain.service", { tool: tool, action: "start" }).then(done);
      if (opt === "install") return run("toolchain.install", { tool: tool }).then(done);
      if (opt === "remove") return run("toolchain.remove", { tool: tool }).then(done);
      if (opt === "logs") return read("toolchain.logs", { tool: tool }).then(function (d) { openDrawer(tool + " logs", '<pre class="log-block">' + esc((d.lines || []).join("\n") || "(no log lines)") + "</pre>"); }).catch(function (e) { toast("error", e.message); });
    },
    "team-charter-edit": function () { openCharterDrawer(); },
    "team-charter-cancel": function () { closeDrawer(); },
    "team-charter-save": function () {
      if (!state.rosterProject) return toast("warn", "Choose a project for the team room first");
      var roles = Array.prototype.map.call(document.querySelectorAll('input[name="charter-role"]:checked'), function (n) { return n.value; });
      var args = {
        project: state.rosterProject,
        north_star: el("charter-north").value.trim(),
        owner: el("charter-owner").value.trim(),
        working_agreement: el("charter-rules").value.split("\n").map(function (l) { return l.trim(); }).filter(Boolean),
        proactive_roles: roles
      };
      run("team.charter.set", args).then(function (outcome) {
        if (!outcome) return null;
        closeDrawer();
        return loadRoster();
      }).then(function () { if (state.view === "agents") renderRoster(); });
    },

    "team-new": function () { TEAM.sel = null; TEAM.data = null; paintTeam(); },
    "team-open": function (n) { TEAM.sel = n.dataset.id; read("team.get", { run: TEAM.sel }).then(function (d) { TEAM.data = d; paintTeam(); }); },
    "team-toggle": function (n) { TEAM.open[n.dataset.id] = !TEAM.open[n.dataset.id]; paintTeam(); },
    "team-start": function () {
      var f = TEAM.form;
      if (!f.goal.trim()) return toast("warn", "Tell the team what you'd like first");
      if (!f.project) return toast("warn", "Choose a project, or a new one");
      var args = { goal: f.goal.trim(), parallel: f.parallel };
      var done = f.done.split("\n").map(function (l) { return l.trim(); }).filter(Boolean);
      if (done.length) args.done_when = done;
      if (f.project === "__new") { if (!f.name.trim()) return toast("warn", "Name the new project"); args.new_project = f.name.trim(); } else args.project = f.project;
      run("team.start", args).then(function (o) {
        if (!o) return;
        TEAM.sel = o.data.id; TEAM.form.goal = ""; TEAM.form.done = ""; TEAM.data = null;
        return loadTeam().then(paintTeam);
      });
    },
    "team-approve": function (n) { run("team.approve_plan", { run: n.dataset.id }).then(function () { return read("team.get", { run: n.dataset.id }); }).then(function (d) { TEAM.data = d; paintTeam(); loadHealth().then(function () { rail(); }); }); },
    "team-pause": function (n) { run("team.pause", { run: n.dataset.id }); },
    "team-resume": function (n) { run("team.resume", { run: n.dataset.id }); },
    "team-cancel": function (n) { run("team.cancel", { run: n.dataset.id }); },
    "team-retry": function (n) { uiPrompt({ title: "Try this task again", text: "What should it do differently? You can leave this empty.", input: { placeholder: "Optional note for the agent" }, confirm: "Retry" }).then(function (note) { if (note === null) return; run("team.retry", { task: n.dataset.id, note: note || undefined }); }); },
    "team-accept": function (n) { uiConfirm({ title: "Accept without the proof?", text: "devd asked for proof that this task worked and did not get it. The final report will say you accepted it by hand.", confirm: "Accept anyway", danger: true }).then(function (yes) { if (yes) run("team.accept", { task: n.dataset.id }); }); },
    "team-say": function (n) { var i = el("team-say"); if (!i.value.trim()) return; var t = i.value.trim(); i.value = ""; var ts = el("team-say-send"); if (ts) ts.disabled = true; run("team.say", { run: n.dataset.id, text: t }); },
    "team-open-from": function (n) { TEAM.sel = n.dataset.id; TEAM.data = null; navigate("team"); },
    "home-team": function () {
      var text = (el("home-ask").value || "").trim(), p = el("home-project").value;
      TEAM.form.goal = takeAttachments(text); if (p) TEAM.form.project = p;
      TEAM.sel = null; TEAM.data = null; navigate("team");
    },
    "copy-output": function (n) {
      var pre = n.dataset.id ? el(n.dataset.id) : null;
      if (!pre) return;
      copyText(pre.textContent || "", function () { toast("ok", "Copied to the clipboard"); });
    },
    "req-fix": function (n) {
      var r = findRequirement(n.dataset.id);
      if (!r) return;
      var fix = r.fix || {};
      if (fix.capability) {
        // The list was read before the fix ran, so it still says the thing is
        // missing. Read it again, and redraw the sheet if it is the one open.
        return Promise.resolve(run(fix.capability, fix.args || {})).then(function () {
          return read("setup.requirements").catch(function () { return null; });
        }).then(function (fresh) {
          if (!fresh) return;
          AI.requirements = fresh;
          if (AI.home) AI.home.requirements = fresh;
          if (document.querySelector(".ui-sheet .req-row")) openSetupSheet();
          if (state.view === "home") render();
        });
      }
      if (fix.view) return navigate(fix.view);
      return ACTIONS["req-how"](n);
    },
    "req-how": function (n) {
      var r = findRequirement(n.dataset.id);
      if (!r) return;
      var fix = r.fix || {};
      var body = '<div class="notice">devd cannot install this one for you. Run this in a terminal:</div>' +
        '<pre class="log-block">' + esc(fix.command || fix.url || "") + "</pre>";
      if (fix.command && fix.url) {
        body += '<div class="row-sub">or download it from <a class="link" href="' + attr(fix.url) + '" target="_blank" rel="noopener noreferrer">' + esc(fix.url) + "</a></div>";
      }
      openDrawer(r.title, body);
    },
    "role-save": function (n) {
      var role = n.dataset.role, models = [];
      [0, 1].forEach(function (i) {
        var p = el("rm-" + role + "-p" + i).value, m = el("rm-" + role + "-m" + i).value.trim();
        if (p && m) models.push({ provider: p, model: m });
      });
      run("models.assign", { role: role, models: models }).then(function (o) { if (o) return loadProviders().then(renderProviders); });
    },
    "env-toggle": function () {
      var to = AI.env && AI.env.mode === "production" ? "local" : "production";
      run("environment.set", { mode: to }).then(function (o) { if (o) return loadHealth().then(function () { return read("environment.get"); }).then(function (e) { AI.env = e; render(); }); });
    },
    // ---- assistant era
    "health-open": function () { openHealth(); loadHealth().then(function () { healthChip(); if (!el("drawer").hidden) openHealth(); }); },
    "health-recheck": function () { AI.healthLoading = true; if (!el("drawer").hidden) openHealth(); loadHealth().then(function () { healthChip(); if (!el("drawer").hidden) openHealth(); if (state.view === "home") loadHome().then(render); }); },
    "health-go": function (n) { closeDrawer(); navigate(n.dataset.view); },
    "health-fix": function (n) {
      var args = {}; try { args = JSON.parse(n.dataset.args || "{}"); } catch (e) {}
      run(n.dataset.cap, args).then(function (o) { if (o) loadHealth().then(function () { healthChip(); openHealth(); }); });
    },
    "home-open": function (n) { DEV.path = n.dataset.path; DEV.ctx = null; DEV.detect = null; navigate("develop"); },
    "home-chat": function (n) {
      var path = (n && n.dataset.path) || "";
      AI.thread = null;
      AI.events = [];
      AI.pending = [];
      AI.lastId = 0;
      AI.todos = [];
      AI.todosAt = 0;
      AI.queued = 0;
      AI.openSteps = {};
      AI.composer = "";
      AI.project = path;
      AI.askProject = path;
      rememberProject(path);
      navigate("assistant").then(function () {
        var input = el("chat-input");
        if (input) input.focus();
      });
    },
    "top-menu": function (n) {
      var name = n.dataset.menu;
      state.topMenu = state.topMenu === name ? "" : name;
      rail();
      if (state.topMenu) {
        var first = document.querySelector('.top-menu .top-pop:not([hidden]) .top-item');
        if (first && n.matches(":focus-visible")) first.focus();
      }
    },
    "reconnect": function () { refresh(); },
    "reload-page": function () { window.location.reload(); },
    "home-prompt": function (n) {
      var box = el("home-ask");
      if (!box) return;
      box.value = n.dataset.text;
      box.focus();
      box.setSelectionRange(box.value.length, box.value.length);
    },
    "home-ask": function () {
      var text = (el("home-ask").value || "").trim();
      if (!text) return toast("warn", "Write what you want done first");
      startChat(takeAttachments(text), el("home-project").value);
    },
    "open-thread": function (n) {
      AI.thread = { id: n.dataset.id, title: "", state: "idle", input_tokens: 0, output_tokens: 0 }; AI.lastId = 0; AI.events = []; AI.pending = []; AI.todos = []; AI.todosAt = 0; AI.queued = 0;
      if (state.view !== "assistant") { state.view = "assistant"; render(); }
      writeRoute("assistant", false);
      pollThread(true).then(function () { renderAssistant(); });
    },
    "chat-new": function () { AI.thread = null; AI.events = []; AI.pending = []; AI.lastId = 0; AI.todos = []; AI.todosAt = 0; AI.queued = 0; writeRoute("assistant", false); renderAssistant(); },
    "chat-plan": function () {
      var rows = (AI.todos || []).map(function (t) {
        var st = t.status || "open";
        var cls = st === "blocked" ? "pill-warn" : st === "done" ? "" : "pill-accent";
        return '<div class="plan-row"><span class="pill ' + cls + '">' + esc(st.replace("_", " ")) + "</span>" +
          '<span class="grow">' + esc(t.title) + '</span><span class="dim mono">' + esc(t.priority || "normal") + "</span></div>" +
          (t.description ? '<div class="dim plan-note">' + esc(t.description) + "</div>" : "");
      }).join("");
      uiSheet("Plan", rows || '<div class="empty-inline">No todos in this project yet.</div>');
    },
    "chat-send": function () {
      var inp = el("chat-input"); var text = (inp.value || "").trim();
      if (!text && !ATTACHMENTS.length) return;
      // The files travel with the message, so the agent can read them.
      text = takeAttachments(text);
      inp.value = ""; AI.composer = "";
      fitComposer();
      if (AI.thread && AI.thread.id) return sendChat({ message: text, thread: AI.thread.id });
      var sel = el("chat-project");
      if (AI.homeMode === "team") { TEAM.form.goal = text; if (sel && sel.value) TEAM.form.project = sel.value; TEAM.sel = null; TEAM.data = null; return navigate("team"); }
      startChat(text, sel ? sel.value : "");
    },
    "chat-stop": function () { if (AI.thread) run("chat.stop", { thread: AI.thread.id }).then(function () { pollThread(false).then(paintChat); }); },
    "open-run": function (n) { openRun(n.dataset.id); },
    "run-team-open": function (n) {
      TEAM.sel = n.dataset.id;
      closeDrawer();
      navigate("team").then(function () { return loadTeam(); }).then(function () { renderTeam(); });
    },
    "run-chat": function (n) {
      AI.thread = { id: n.dataset.id, title: "", state: "idle", input_tokens: 0, output_tokens: 0 };
      AI.lastId = 0; AI.events = []; AI.pending = []; AI.todos = []; AI.todosAt = 0; AI.queued = 0; AI.run = null;
      closeDrawer();
      navigate("assistant").then(function () { return pollThread(true); }).then(function () { renderAssistant(); });
    },
    "run-cancel": function (n) {
      n.disabled = true;
      request("/api/runs/" + encodeURIComponent(n.dataset.id) + "/cancel", { method: "POST", body: "{}" }).then(function (res) {
        if (!res.ok) { toast("error", errorOf(res.body, "the run could not be stopped")); n.disabled = false; return; }
        toast("ok", "Stopping. The run closes when the executor reaches a safe point.");
        if (RUN.id === n.dataset.id) loadRun(RUN.id).then(paintRunSheet);
        if (state.view === "assistant" && AI.thread) pollThread(false).then(paintChat).catch(function () {});
      });
    },
    "run-retry": function (n) {
      n.disabled = true;
      request("/api/runs/" + encodeURIComponent(n.dataset.id) + "/retry", { method: "POST", body: "{}" }).then(function (res) {
        if (!res.ok) { toast("error", errorOf(res.body, "the run could not be continued")); n.disabled = false; return; }
        toast("ok", "Continuing the run. Its recorded work is not replayed.");
        if (RUN.id === n.dataset.id) loadRun(RUN.id).then(paintRunSheet);
        if (state.view === "assistant" && AI.thread) pollThread(false).then(paintChat).catch(function () {});
      });
    },
    "run-older": function (n) {
      var first = RUN.events.length ? RUN.events[0].seq : 0;
      if (!first) return;
      n.disabled = true;
      request("/api/runs/" + encodeURIComponent(n.dataset.id) + "/events?before=" + encodeURIComponent(first) + "&limit=200").then(function (res) {
        if (!res.ok) { toast("error", errorOf(res.body, "earlier events could not be read")); return; }
        var items = (res.body && res.body.items) || [];
        RUN.events = items.concat(RUN.events);
        RUN.hasOlder = items.length >= 200;
        paintRunSheet();
      });
    },
    "mention-pick": function (n) {
      var it = MENTIONS.items[Number(n.dataset.index)];
      if (it) applyMention(it);
    },
    "step-toggle": function (n) { AI.openSteps[n.dataset.id] = !AI.openSteps[n.dataset.id]; paintChat(); },
    "req-decide": function (n) {
      closeDrawer();
      var approve = n.dataset.approve === "1";
      n.disabled = true;
      run("approval.decide", { id: n.dataset.id, approve: approve }).then(function () {
        return loadHealth();
      }).then(function () {
        if (state.view === "inbox") return loadInbox().then(render);
        if (state.view === "home") return loadHome().then(render);
        if (AI.thread) return pollThread(false).then(paintChat);
      });
    },
    "inbox-reply": function (n) {
      closeDrawer();
      var inp = el("reply-" + n.dataset.id);
      var args = { id: n.dataset.id };
      if (inp) { if (!inp.value.trim()) return toast("warn", "Write an answer first"); args.reply = inp.value.trim(); }
      run("inbox.reply", args).then(function () { return loadHealth(); }).then(function () { return state.view === "home" ? loadHome().then(render) : loadInbox().then(render); });
    },
    "prov-new": function (n) {
      var pr = (AI.presets || []).filter(function (x) { return x.id === n.dataset.preset; })[0] || {};
      var taken = (AI.providers || []).map(function (p) { return p.id; });
      var id = pr.id || "provider", k = 2; while (taken.indexOf(id) !== -1) id = (pr.id || "provider") + "-" + k++;
      AI.provForm = { preset: pr.id, kind: pr.kind, label: pr.label, id: id, base_url: pr.base_url || "", key_env: pr.key_env || "", default_model: pr.default_model || "", make_default: !(AI.providers || []).length, editing: false };
      renderProviders();
      var f = document.querySelector('[data-pf="api_key"]'); if (f) f.focus();
    },
    "prov-edit": function (n) {
      var p = (AI.providers || []).filter(function (x) { return x.id === n.dataset.id; })[0]; if (!p) return;
      // The preset is only set when the provider really is one of the known
      // presets. A custom provider has no preset, and saying so is what lets
      // it be edited.
      AI.provForm = { preset: p.preset || null, kind: p.kind, label: p.label, id: p.id, base_url: p.base_url, key_env: p.key_env || "", default_model: p.default_model || "", make_isDefault: false, editing: true };
      renderProviders();
    },
    "prov-cancel": function () { AI.provForm = null; renderProviders(); },
    "prov-save": function () {
      var f = AI.provForm; if (!f) return;
      var q = function (k) { var e = document.querySelector('[data-pf="' + k + '"]'); return e ? (e.type === "checkbox" ? e.checked : e.value.trim()) : ""; };
      var args = { id: f.editing ? f.id : q("id"), kind: q("kind") || f.kind, label: f.label, base_url: q("base_url"), default_model: q("default_model"), make_default: q("make_default") };
      if (f.preset) args.preset = f.preset;
      if (q("key_env")) args.key_env = q("key_env");
      var key = document.querySelector('[data-pf="api_key"]'); if (key && key.value) args.api_key = key.value;
      run("provider.save", args).then(function (o) {
        if (key) key.value = "";
        if (!o) return;
        AI.provForm = null;
        return loadProviders().then(function () { renderProviders(); loadHealth().then(healthChip); });
      });
    },
    "prov-test": function (n) { providerCall(n.dataset.id, "provider.test"); },
    "prov-models": function (n) { providerCall(n.dataset.id, "provider.models"); },
    "prov-verify": function (n) {
      var p = (AI.providers || []).filter(function (x) { return x.id === n.dataset.id; })[0];
      if (!p) return;
      if (!p.default_model) return toast("warn", "Choose a model for this provider first");
      providerCall(p.id, "provider.verify", { model: p.default_model });
    },
    "prov-discover": function () {
      read("provider.discover_local").then(function (found) {
        AI.localFound = found || [];
        renderProviders();
        var up = AI.localFound.filter(function (f) { return f.running; }).length;
        toast(up ? "ok" : "info", up ? up + " local server(s) found" : "No local model server is running");
      }).catch(function (e) { toast("error", e.message); });
    },
    "prov-default": function (n) { run("provider.default", { id: n.dataset.id }).then(function () { return loadProviders(); }).then(renderProviders); },
    "prov-remove": function (n) { run("provider.remove", { id: n.dataset.id }).then(function (o) { if (o) return loadProviders().then(function () { renderProviders(); loadHealth().then(healthChip); }); }); },
    "prov-pick": function (n) { setProviderModel(n.dataset.id, n.dataset.model); },
    "prov-use-model": function (n) {
      var d = (AI.providers || []).filter(function (p) { return p.kind === "anthropic"; })[0];
      if (!d) return toast("warn", "Add an Anthropic provider first");
      setProviderModel(d.id, n.dataset.model);
    },
    "models-refresh": function () { run("models.refresh_official", {}).then(function () { return loadProviders(); }).then(renderProviders); },
    "create-pick": function (n) { pickStack(n.dataset.id); },
    "create-back": function () { CREATE.id = null; CREATE.def = null; CREATE.plan = null; renderCreate(); },
    "create-run": function () {
      if (CREATE.running) return;
      CREATE.running = true; CREATE.report = null; CREATE.log = ["starting…"]; CREATE.runId = "ui-" + Date.now().toString(36);
      var req = createRequest(); req.run_id = CREATE.runId;
      paintReview(); paintProgress(); pollCreateLog();
      request("/api/invoke", { method: "POST", body: JSON.stringify({ id: "project.init", args: req, dry_run: false }) }).then(function (res) {
        CREATE.running = false;
        if (!res.ok) { CREATE.report = { status: "failed", error: errorOf(res.body, "project.init failed"), steps: [], checks: [], next: [] }; }
        else { CREATE.report = res.body.outcome.data; }
        return read("stacks.log", { run_id: CREATE.runId, lines: 80 }).then(function (d) { CREATE.log = d.lines || CREATE.log; }).catch(function () {});
      }).then(function () { toast(CREATE.report.status === "ok" ? "ok" : "error", "project.init: " + CREATE.report.status); paintReview(); paintProgress(); });
    },
    "create-cancel": function () { act("stacks.cancel", { run_id: CREATE.runId }); },
    "open-terminal": function (n) { run("project.terminal", { path: n.dataset.path }); },
    "open-develop": function (n) { DEV.path = n.dataset.path; DEV.ctx = null; DEV.detect = null; navigate("develop"); },
    "tool-install": function (n) { act("toolchain.install", { tool: n.dataset.id }).then(function () { if (state.view === "create") scheduleValidate(); }); },
    "tool-remove": function (n) { act("toolchain.remove", { tool: n.dataset.id }); },
    "svc": function (n) { act("toolchain.service", { tool: n.dataset.id, action: n.dataset.act }); },
    "svc-logs": function (n) { read("toolchain.logs", { tool: n.dataset.id }).then(function (d) { openDrawer(n.dataset.id + " logs", '<pre class="log-block">' + esc((d.lines || []).join("\n") || "empty") + "</pre>"); }).catch(function (e) { toast("error", e.message); }); },
    "devices": function () { read("toolchain.devices").then(function (d) { openDrawer("Android devices", d.length ? d.map(function (x) { return '<div class="kv"><dt class="mono">' + esc(x.serial) + "</dt><dd>" + esc(x.state) + " " + esc(x.note) + "</dd></div>"; }).join("") : '<div class="empty">No devices. Connect one with USB debugging enabled, or start an emulator.</div>'); }).catch(function (e) { toast("error", e.message); }); },
    "eas-projects": function () { read("toolchain.eas_projects").then(function (d) { var list = d.projects || []; openDrawer("Expo projects", list.length ? list.map(function (x) { return '<div class="kv"><dt class="mono">' + esc(x.slug) + "</dt><dd>" + esc(x.name || "") + (x.owner ? " · " + esc(x.owner) : "") + "</dd></div>"; }).join("") : '<div class="empty">' + esc(d.message || "No Expo projects. Sign in with `eas login` first.") + "</div>"); }).catch(function (e) { toast("error", e.message); }); },
    "dev-open": function (n) { detectProject(n.dataset.path); },
    "dev-pick": function () {
      openPicker({
        title: "Choose a project folder",
        note: "Pick the folder that holds the project. devd will look at what is inside it.",
        confirm: "Use this folder",
        intent: "project",
        start: DEV.importPath
      }).then(function (path) {
        if (!path) return;
        DEV.importPath = path;
        var input = el("dev-import");
        if (input) input.value = path;
        return ACTIONS["dev-detect"]();
      });
    },
    "dev-detect": function () {
      var p = readInput("dev-import"); DEV.importPath = p; if (!p) return toast("warn", "Say where the project is, or choose a folder");
      DEV.busy = true; renderDevelop();
      read("project.detect", { path: p }).then(function (c) { DEV.detect = c; DEV.path = p; DEV.busy = false; renderDevelop(); }).catch(function (e) { DEV.busy = false; DEV.error = e.message; renderDevelop(); });
    },
    "dev-import": function () {
      var p = readInput("dev-import"); DEV.importPath = p; if (!p) return toast("warn", "Say where the project is, or choose a folder");
      act("project.import", { path: p }).then(function (o) { if (o) { DEV.detect = null; DEV.ctx = null; DEV.path = p; return loadDevelop().then(renderDevelop); } });
    },
    "task-run": function (n) {
      var id = n.dataset.task, destructive = n.dataset.risk === "destructive";
      DEV.log = "running " + id + "…"; renderDevelop();
      run(destructive ? "project.run_destructive" : "project.run", { path: DEV.path, task: id }).then(function (o) {
        if (o && o.data) DEV.log = (o.data.output_tail || []).join("\n") || o.message; else DEV.log = null;
        return loadDevelop();
      }).then(renderDevelop);
    },
    "proc-log": function (n) { read("task.logs", { name: n.dataset.name }).then(function (d) { openDrawer(n.dataset.name, '<pre class="log-block">' + esc(d.lines.join("\n") || "empty") + "</pre>"); }).catch(function (e) { toast("error", e.message); }); },
    "proc-stop": function (n) { act("task.stop", { name: n.dataset.name }); },


    refresh: function () { refresh(); },

    palette: function () { openPalette(); },

    "palette-pick": function (node) { paletteActivate(Number(node.dataset.index)); },

    "dismiss-toast": function (node) {
      dismissToast(node.closest(".toast"));
    },

    "cap-page": function (node) {
      state.caps.page = Math.max(0, Number(node.dataset.page) || 0);
      renderCapabilities();
    },

    "audit-page": function (node) {
      state.auditPage = Math.max(0, Number(node.dataset.page) || 0);
      renderActivity();
    },

    "skill-read": function (n) {
      read("skills.get", { name: n.dataset.name }).then(function (d) {
        openDrawer(n.dataset.name, '<div class="kv"><dt>Kind</dt><dd>' + esc(d.kind) + "</dd></div>" +
          '<div class="row-sub">' + esc(d.description) + "</div>" +
          '<pre class="log-block">' + esc(d.body || "") + "</pre>");
      }).catch(function (e) { toast("error", e.message); });
    },
    "skill-remove": function (n) {
      act("skills.delete", { name: n.dataset.name }).then(function () { return loadSkills(); }).then(render);
    },
    "skill-install": function () {
      var src = el("skill-source");
      var value = src ? src.value.trim() : "";
      if (!value) return toast("warn", "Give the repository as owner/name");
      act("skills.install", { source: value }).then(function () {
        toast("ok", "installed");
        return loadSkills();
      }).then(render);
    },
    "skill-import": function (n) {
      act("skills.import", { from: n.dataset.agent }).then(function () {
        return loadSkills();
      }).then(render);
    },
    "skill-browse": function () {
      window.open("https://skills.sh", "_blank", "noopener");
    },
    "close-drawer": function () { closeDrawer(); },
    "msg-copy": function (n) {
      var ev = AI.events.filter(function (e) { return String(e.id) === n.dataset.event; })[0];
      var b = ev && (ev.data.content || [])[+n.dataset.block];
      copyText((b && b.text) || "", function () {
        var old = n.innerHTML;
        n.innerHTML = '<svg class="icon" aria-hidden="true"><use href="#i-check"></use></svg>Copied';
        setTimeout(function () { n.innerHTML = old; }, 1400);
      });
    },
    "msg-fork": function (n) {
      if (!AI.thread) return;
      run("chat.fork", { thread: AI.thread.id, event: +n.dataset.event }).then(function (o) {
        var forked = o && o.data && o.data.thread;
        if (!forked) return;
        toast("ok", "Forked into a new conversation. The original is unchanged.");
        return read("chat.list").then(function (list) {
          AI.threads = list || [];
          ACTIONS["open-thread"]({ dataset: { id: forked.id } });
        });
      });
    },
    "msg-pin": function (n) {
      if (!AI.thread) return;
      var tid = AI.thread.id, eid = +n.dataset.event;
      var chapter = (AI.chapters || []).filter(function (c) { return c.event_id === eid; })[0];
      var refresh = function () {
        return read("chat.chapters", { thread: tid }).then(function (d) {
          AI.chapters = (d && d.chapters) || [];
          paintChat();
        });
      };
      if (chapter) return run("chat.unpin", { id: chapter.id }).then(function (o) { if (o) return refresh(); });
      var ev = AI.events.filter(function (e) { return e.id === eid; })[0];
      var b = ev && (ev.data.content || [])[+n.dataset.block];
      var guess = uiSummary(String((b && b.text) || "").replace(/^#+\s*/, ""), 60) || "Chapter";
      uiPrompt({ title: "Pin as chapter", text: "Name this point in the conversation. Chapters appear in the Chapters list at the top.", input: { value: guess, placeholder: "Chapter title" }, confirm: "Pin" }).then(function (title) {
        if (!title) return;
        return run("chat.pin", { thread: tid, event: eid, title: title }).then(function (o) { if (o) return refresh(); });
      });
    },
    "chapters-open": function () {
      var rows = (AI.chapters || []).map(function (c) {
        return uiRow({
          title: esc(c.title), sub: esc(ago(c.created_at) + " ago"),
          open: 'data-action="chapter-jump" data-event="' + c.event_id + '"',
          actions: '<button type="button" class="btn btn-mini" data-action="chapter-jump" data-event="' + c.event_id + '">Jump</button><button type="button" class="btn btn-mini btn-ghost" data-action="chapter-remove" data-id="' + attr(c.id) + '">Remove</button>',
          hover: true
        });
      });
      uiSheet("Chapters", rows.length ? uiList(rows) : uiEmpty("No chapters yet", "Pin a reply with Pin as chapter to mark a point you will want to find again."));
    },
    "chapter-jump": function (n) {
      closeDrawer();
      var node = document.querySelector('.msg[data-event="' + n.dataset.event + '"]');
      if (!node) return toast("warn", "That message is not on screen");
      node.scrollIntoView({ block: "start", behavior: "smooth" });
      node.classList.add("msg-flash");
      setTimeout(function () { node.classList.remove("msg-flash"); }, 1800);
    },
    "chapter-remove": function (n) {
      run("chat.unpin", { id: n.dataset.id }).then(function (o) {
        if (!o) return;
        AI.chapters = (AI.chapters || []).filter(function (c) { return c.id !== n.dataset.id; });
        paintChat();
        ACTIONS["chapters-open"]();
      });
    },
    "copy-code": function (n) {
      var pre = n.closest(".md-code").querySelector("pre");
      var text = pre ? pre.innerText : "";
      var done = function () { n.textContent = "Copied"; setTimeout(function () { n.textContent = "Copy"; }, 1400); };
      if (navigator.clipboard && navigator.clipboard.writeText) navigator.clipboard.writeText(text).then(done, done);
      else { var t = document.createElement("textarea"); t.value = text; document.body.appendChild(t); t.select(); try { document.execCommand("copy"); } catch (e) { /* nothing to do */ } t.remove(); done(); }
    },
    "chat-delete": function (n) {
      var id = n.dataset.id;
      run("chat.delete", { thread: id }).then(function (o) {
        if (!o) return;
        if (AI.thread && AI.thread.id === id) { AI.thread = null; AI.events = []; AI.pending = []; AI.lastId = 0; }
        return loadAssistant().then(render);
      });
    },
    "chats-prune": function () {
      run("chat.prune", {}).then(function (o) {
        if (!o) return;
        if (AI.thread && !(AI.threads || []).some(function (t) { return t.id === AI.thread.id; })) AI.thread = null;
        AI.thread = null; AI.events = []; AI.pending = []; AI.lastId = 0;
        return loadAssistant().then(render);
      });
    },
    "team-delete": function (n) {
      var id = n.dataset.id;
      run("team.delete", { run: id }).then(function (o) {
        if (!o) return;
        if (TEAM.sel === id) { TEAM.sel = null; TEAM.data = null; }
        return refresh();
      });
    },
    "team-prune": function () {
      run("team.prune", {}).then(function (o) {
        if (!o) return;
        TEAM.sel = null; TEAM.data = null;
        return refresh();
      });
    },
    "inbox-clear": function () {
      run("inbox.clear", {}).then(function (o) { if (o) return loadInbox().then(render); });
    },
    "open-audit": function (n) { openAuditSheet(n.dataset.at, n.dataset.cap); },
    "audit-filter": function (n) { state.auditFilter = n.dataset.filter; state.auditPage = 0; render(); },
    "team-report": function (n) { var d = TEAM.data; if (d && d.run && d.run.id === n.dataset.id) uiSheet(d.run.title, uiBody(d.run.summary)); },
    "open-setup": function () { openSetupSheet(); },
    "open-message": function (n) { openMessageSheet(n.dataset.id); },
    "open-approval": function (n) { openApprovalSheet(n.dataset.id); },
    "filter-clear": function (n) {
      var key = n.dataset.key;
      if (!key) return;
      state.filters[key] = "";
      render();
      var again = document.querySelector('[data-filter="' + key + '"]');
      if (again) again.focus();
    },

    // ------------------------------------------------------------- projects

    "add-project": function () {
      openPicker({
        title: "Add a project",
        note: "Choose the folder that holds the project. devd looks at what is inside it before anything is registered.",
        confirm: "Add this project",
        intent: "project"
      }).then(function (path) {
        if (!path) return;
        return addProjectAt(path);
      });
    },

    "pin-project": function (node) {
      togglePin(node.dataset.path);
    },

    "project-sort": function (node) {
      state.projectSort = node.dataset.sort;
      renderProjects();
    },

    "projects-reload": function () {
      state.projectsLoaded = false;
      state.projectsError = "";
      renderProjects();
      loadProjects().then(render);
    },

    "site-filter": function (node) {
      state.siteFilter = node.dataset.filter || "all";
      renderSites();
    },

    // ------------------------------------------------------------- picker

    "picker-goto": function (node) {
      pickerGo(node.dataset.path);
    },

    "picker-pick-file": function (node) {
      // A file cannot be a project, but it may sit next to one; going to its
      // folder is the useful thing to do.
      var path = node.dataset.path || "";
      var cut = path.replace(/[\\/][^\\/]+$/, "");
      if (cut) pickerGo(cut);
    },

    "picker-up": function () {
      if (picker.listing && picker.listing.parent) pickerGo(picker.listing.parent);
    },

    "picker-native": function () {
      request("/api/pick-folder", {
        method: "POST",
        body: JSON.stringify({
          title: picker.title,
          start: picker.listing ? picker.listing.path : picker.selected
        })
      }).then(function (res) {
        if (res.status === 501) {
          toast("info", "This window has no native folder dialog", "Use the browser below to move through your device.");
          return;
        }
        if (!res.ok) {
          toast("error", errorOf(res.body, "the folder dialog could not be shown"));
          return;
        }
        if (!res.body || !res.body.path) return;
        return pickerGo(res.body.path);
      });
    },

    "picker-choose": function () {
      var path = picker.selected;
      if (!path) {
        toast("warn", "Choose a folder first");
        return;
      }
      closePicker(path);
    },

    "picker-cancel": function () { closePicker(null); },

    "picker-close": function () { closePicker(null); },

    "add-form": function () {
      state.addOpen = true;
      renderProjects();
    },

    "add-cancel": function () { state.addOpen = false; renderProjects(); },

    "stack-up": function () { act("stack.up", {}); },

    "stack-down": function () { act("stack.down", {}); },

    "install-service": function (node) {
      toast("info", "installing " + node.dataset.name + " — the vendor download can take a minute");
      act("service.install", { name: node.dataset.name, version: null });
    },

    "adopt-found": function (node) {
      uiConfirm({ title: "Run " + node.dataset.name + " with devd?", text: "Found in " + node.dataset.path + ". The folder is not changed; devd keeps its own data and configuration.", confirm: "Run it with devd" }).then(function (yes) {
        if (yes) act("service.adopt", { name: node.dataset.name, path: node.dataset.path });
      });
    },

    "schedule-create": function () {
      var f = SCHED.form;
      var params = {};
      (f.params || "").split("\n").forEach(function (line) {
        var i = line.indexOf("=");
        if (i > 0) params[line.slice(0, i).trim()] = line.slice(i + 1).trim();
      });
      var args = { name: f.name.trim(), spec: f.spec.trim(), project: f.project };
      if (f.template) { args.template = f.template; args.params = params; } else args.goal = f.goal.trim();
      var done = (f.done || "").split("\n").map(function (l) { return l.trim(); }).filter(Boolean);
      if (done.length) args.done_when = done;
      if (f.waive) args.auto_approve = true;
      if (!args.name || !args.project) return toast("warn", "Give it a name and choose a project");
      run("schedule.create", args).then(function (o) {
        if (!o) return;
        SCHED.form.name = ""; SCHED.form.params = ""; SCHED.form.goal = ""; SCHED.form.done = "";
        return loadSchedules().then(renderSchedules);
      });
    },

    "schedule-run": function (node) { run("schedule.run_now", { name: node.dataset.name }).then(function (o) { if (o) loadSchedules().then(renderSchedules); }); },

    "schedule-toggle": function (node) { run("schedule.enable", { name: node.dataset.name, enabled: node.dataset.on !== "1" }).then(function () { return loadSchedules().then(renderSchedules); }); },

    "schedule-delete": function (node) { uiConfirm({ title: "Delete the schedule " + node.dataset.name + "?", text: "Runs it already started are not touched.", confirm: "Delete", danger: true }).then(function (yes) { if (yes) run("schedule.delete", { name: node.dataset.name }).then(function () { return loadSchedules().then(renderSchedules); }); }); },

    "mcp-edit": function (node) {
      var s = (MCP.servers || []).filter(function (x) { return x.name === node.dataset.name; })[0];
      if (!s) return;
      var args = (s.args || []).join("\n");
      var trust = [["ask", "Ask me each time"], ["allow", "Let agents use it without asking"], ["deny", "Agents may not use it"]]
        .map(function (o) { return '<option value="' + o[0] + '"' + (s.trust === o[0] ? " selected" : "") + ">" + esc(o[1]) + "</option>"; }).join("");
      uiSheet("Edit " + s.name,
        '<div class="stack">' +
        '<div class="field"><label class="field-label">Program <span class="dim">(stdio)</span></label><input class="input mono" id="mcp-edit-command" value="' + attr(s.command || "") + '" spellcheck="false"></div>' +
        '<div class="field"><label class="field-label">Address <span class="dim">(http)</span></label><input class="input mono" id="mcp-edit-url" value="' + attr(s.url || "") + '" spellcheck="false"></div>' +
        '<div class="field"><label class="field-label">Arguments <span class="dim">(one per line)</span></label><textarea class="input mono" id="mcp-edit-args" rows="5" spellcheck="false">' + esc(args) + "</textarea></div>" +
        (args.indexOf("[redacted]") >= 0 ? '<div class="notice notice-warn">A credential-looking argument is shown as [redacted] and is kept unchanged. To replace it, remove the server and add it again.</div>' : "") +
        '<div class="inline"><div class="field grow"><label class="field-label">Timeout <span class="dim">(seconds)</span></label><input class="input" id="mcp-edit-timeout" value="' + attr(String(s.timeout_secs)) + '" inputmode="numeric"></div>' +
        '<div class="field grow"><label class="field-label">Working directory</label><input class="input mono" id="mcp-edit-cwd" value="' + attr(s.cwd || "") + '" spellcheck="false"></div></div>' +
        '<div class="inline"><div class="field grow"><label class="field-label">Agents</label><select class="select" id="mcp-edit-trust">' + trust + "</select></div>" +
        '<label class="check"><input type="checkbox" id="mcp-edit-enabled"' + (s.enabled ? " checked" : "") + "> Enabled</label></div>" +
        '<p class="dim">Environment: ' + esc((s.env_keys || []).join(", ") || "none") + ". It is kept as it is; remove and re-add the server to replace it.</p>" +
        '<div class="ui-actions"><button type="button" class="btn btn-primary" data-action="mcp-edit-save" data-name="' + attr(s.name) + '">Save</button></div>' +
        "</div>");
    },
    "mcp-edit-save": function (node) {
      var name = node.dataset.name;
      var command = readInput("mcp-edit-command");
      var url = readInput("mcp-edit-url");
      var args = (el("mcp-edit-args") ? el("mcp-edit-args").value : "").split(/\r?\n/).map(function (x) { return x.trim(); }).filter(Boolean);
      var keepArgs = args.some(function (a) { return a.indexOf("[redacted]") >= 0; });
      var cwd = readInput("mcp-edit-cwd");
      var payload = {
        name: name,
        command: command || null,
        url: url || null,
        timeout_secs: parseInt(readInput("mcp-edit-timeout"), 10) || 120,
        cwd: cwd || null,
        trust: readInput("mcp-edit-trust") || "ask",
        enabled: checked("mcp-edit-enabled")
      };
      if (!keepArgs) payload.args = args;
      run("mcp.update", payload).then(function (o) {
        if (!o) return;
        closeDrawer();
        toast("ok", "Changed " + name);
        return loadMcp().then(renderMcp);
      }).catch(function (e) { toast("error", e.message); });
    },
    "mcp-add": function () {      var name = readInput("mcp-name");
      var command = readInput("mcp-command");
      var url = readInput("mcp-url");
      var args = (el("mcp-args") ? el("mcp-args").value : "").split(/\r?\n/).map(function (x) { return x.trim(); }).filter(Boolean);
      var env = {};
      (el("mcp-env") ? el("mcp-env").value : "").split(/\r?\n/).forEach(function (line) {
        var i = line.indexOf("=");
        if (i > 0) { var k = line.slice(0, i).trim(); if (k) env[k] = line.slice(i + 1); }
      });
      var cwd = readInput("mcp-cwd");
      var timeout = parseInt(readInput("mcp-timeout"), 10);
      if (!name) return toast("error", "Give the server a name.");
      if (name !== name.toLowerCase() || !/^[a-z0-9-]+$/.test(name)) return toast("error", "A name is lower-case letters, digits and `-`.");
      if (!command && !url) return toast("error", "Give a program to start (stdio), or an http address.");
      run("mcp.add", { name: name, command: command || null, args: args, url: url || null, env: env, cwd: cwd || null, timeout_secs: timeout || 120 })
        .then(function (o) { if (!o) return; toast("ok", "Added " + name + ". Use Test to discover its tools."); return loadMcp().then(renderMcp); })
        .catch(function (e) { toast("error", e.message); });
    },
    "mcp-import": function () {
      run("mcp.import", {}).then(function (o) {
        if (!o) return;
        var imported = (o.imported || []).length, skipped = (o.skipped || []).length;
        toast(imported ? "ok" : "info", imported + " imported" + (skipped ? ", " + skipped + " skipped" : "") + ((o.files || []).length ? " from " + (o.files || []).join(", ") : ". No configuration file was found."));
        return loadMcp().then(renderMcp);
      }).catch(function (e) { toast("error", e.message); });
    },
    "mcp-test": function (node) {
      run("mcp.test", { name: node.dataset.name }).then(function (o) {
        if (o) toast("ok", node.dataset.name + " answered with " + ((o.tools || []).length) + " tool(s).");
        return loadMcp().then(renderMcp);
      }).catch(function (e) { toast("error", e.message); loadMcp().then(renderMcp); });
    },
    "mcp-enable": function (node) {
      run("mcp.enable", { name: node.dataset.name, enabled: node.dataset.on !== "1" }).then(function () { return loadMcp().then(renderMcp); }).catch(function (e) { toast("error", e.message); });
    },
    "mcp-trust": function (node) {
      run("mcp.trust", { name: node.dataset.name, trust: node.value }).then(function () { return loadMcp().then(renderMcp); }).catch(function (e) { toast("error", e.message); loadMcp().then(renderMcp); });
    },
    "mcp-remove": function (node) {
      uiConfirm({ title: "Remove the MCP server " + node.dataset.name + "?", text: "Its tools stop being offered and its private environment file is deleted. The program itself is not uninstalled.", confirm: "Remove", danger: true })
        .then(function (yes) { if (yes) run("mcp.remove", { name: node.dataset.name }).then(function () { return loadMcp().then(renderMcp); }); });
    },

    "start-service": function (node) { act("service.start", { name: node.dataset.name }); },

    "service-startup": function (node) {
      var wanted = !!node.checked;
      run("service.set_startup", { name: node.dataset.name, enabled: wanted }, { quiet: true }).then(function (o) {
        if (!o) { node.checked = !wanted; return null; }
        return loadServer().then(renderServices);
      });
    },

    "web-use": function (node) {
      run("web.use", { server: node.dataset.name }).then(function (o) {
        if (!o) return null;
        return loadSession().catch(function () {}).then(function () { return loadServer(); }).then(renderServices);
      });
    },

    "databases-start": function () {
      run("service.start", { name: "mariadb" }).then(function (outcome) {
        if (!outcome) return null;
        OPS.error = "";
        OPS.errorTab = "";
        OPS.data.db = null;
        return loadServer();
      }).then(function () { return opsLoad("db"); }).then(render);
    },

    "stop-service": function (node) { act("service.stop", { name: node.dataset.name }); },

    "restart-service": function (node) { act("service.restart", { name: node.dataset.name }); },

    "remove-service": function (node) { act("service.remove", { name: node.dataset.name }); },

    "service-logs": function (node) {
      var name = node.dataset.name;
      openDrawer("service.logs " + name, '<div class="dim">Loading…</div>');
      read("service.logs", { name: name, lines: 400 })
        .then(function (data) {
          var lines = (data && data.lines) || [];
          var text = lines.length ? lines.join("\n") : "The log is empty.";
          openDrawer("service.logs " + name, '<pre class="log-block">' + esc(text) + "</pre>");
        })
        .catch(function (e) {
          openDrawer("service.logs " + name, '<div class="notice notice-bad">' + esc(e.message) + "</div>");
        });
    },

    "member-assign": function (node) { openAssign(node.dataset.id); },

    "member-assign-submit": function (node) { submitAssign(node.dataset.id); },

    "new-site": function () {
      // The form lives on the Sites page; from anywhere else the button takes you there with it open.
      if (state.view !== "sites") { state.siteFormOpen = true; navigate("sites").then(function () { if (el("site-name")) el("site-name").focus(); }); return; }
      state.siteFormOpen = !state.siteFormOpen;
      render();
      if (state.siteFormOpen && el("site-name")) el("site-name").focus();
    },

    "site-serving": function (node) {
      run("site.set_serving", { name: node.dataset.name, serving: node.value || null }).then(function (o) {
        if (o) return loadServer().then(render);
        return null;
      });
    },

    "site-clear-php": function (node) {
      run("site.set_php", { name: node.dataset.name, version: null }).then(function (o) {
        if (o) return loadServer().then(render);
        return null;
      });
    },

    "site-php": function (node) {
      var version = node.value || null;
      run("site.set_php", { name: node.dataset.name, version: version }).then(function (o) {
        if (o) return loadServer().then(render);
        return null;
      });
    },

    "create-site": function () {
      var name = readInput("site-name");
      if (!name) return toast("error", "a site needs a name");
      var host = readInput("site-host");
      var args = { name: name, host: host || null, tls: checked("site-tls"), db: checked("site-db"), upstream: readInput("site-upstream") || null };
      act("site.create", args).then(function (outcome) {
        if (outcome) {
          state.siteFormOpen = false;
          render();
        }
      });
    },

    "remove-site": function (node) {
      var name = node.dataset.name;
      act("site.remove", { name: name }).then(function (o) {
        if (o && state.view === "site" && SITE.name === name) {
          SITE.name = "";
          navigate("sites");
        }
      });
    },
    "share-site": function (node) { act("share.start", { name: node.dataset.name }); },
    "unshare-site": function (node) { act("share.stop", { name: node.dataset.name }); },

    "open-url": function (node) {
      post("/api/open-url", { url: node.dataset.url }).then(function (res) {
        if (!res.ok) toast("error", errorOf(res.body, "could not open the URL"));
      });
    },

    "shell-path": function (node) {
      run(node.dataset.mode === "disable" ? "shell.path_disable" : "shell.path_enable", {}).then(function (o) {
        if (o) toast("info", node.dataset.mode === "disable" ? "devd's folder is off your PATH" : "open a new terminal to use php and composer");
        return loadShell();
      });
    },
    "install-composer": function () {
      toast("info", "downloading Composer and checking it against the published digest");
      run("composer.install", {}).then(function () { return loadShell(); });
    },
    "install-runtime": function (node) {
      toast("info", "installing " + node.dataset.kind + " — the vendor download can take a minute");
      act("runtime.install", { kind: node.dataset.kind, version: "latest" }).then(function () {
        SHELL.data = null;
        if (state.view === "runtimes") renderRuntimes();
      });
    },

    "remove-runtime": function (node) {
      act("runtime.remove", { kind: node.dataset.kind, version: node.dataset.version });
    },

    "use-runtime": function (node) { act("runtime.use", { version: node.dataset.version }); },

    "ca-trust": function (node) {
      act("tls.trust", { machine: node.dataset.machine === "true" });
    },

    "ca-untrust": function () {
      act("tls.untrust", { machine: false });
    },

    "cert-form": function () {
      state.certFormOpen = !state.certFormOpen;
      render();
      if (state.certFormOpen && el("cert-host")) el("cert-host").focus();
    },

    "issue-cert": function () {
      var host = readInput("cert-host");
      if (!host) return toast("error", "a certificate needs a hostname");
      act("tls.issue", { host: host }).then(function (outcome) {
        if (outcome) {
          state.certFormOpen = false;
          render();
        }
      });
    },

    "sync-hosts": function () {
      act("hosts.sync", { elevate: true }).then(function (outcome) {
        if (outcome) render();
      });
    },

    "process-refresh": function () { loadProcesses().then(renderProcesses); },
    "approve-project": function () { approveProject(); },
    "process-start": function (node) { processAction("start", node.dataset.name, false); },
    "process-stop": function (node) { processAction("stop", node.dataset.name, false); },
    "process-restart": function (node) { processAction("restart", node.dataset.name, false); },
    "process-start-all": function () { processAction("start", "", true); },
    "process-stop-all": function () { processAction("stop", "", true); },
    "process-logs": function (node) { openProcessLogs(node.dataset.name); },
    "close-process-logs": function () { state.processes.selected = null; state.processes.log = null; renderProcesses(); },

    "shared-tab": function (node) { state.shared.tab = node.dataset.tab; renderShared(); },
    "todo-open": function () { state.shared.todoOpen = true; renderShared(); },
    "todo-cancel": function () { state.shared.todoOpen = false; renderShared(); },
    "todo-create": function () { createTodo(); },
    "todo-done": function (node) { completeTodo(node.dataset.id, node.dataset.rev); },
    "todo-status": function () { },
    "todo-reopen": function (node) { reopenTodo(node.dataset.id, node.dataset.rev); },
    "todo-delete": function (node) { deleteTodo(node.dataset.id); },
    "note-open": function (node) { openNote(node.dataset.name); },
    "note-open-create": function () { state.shared.noteOpen = true; renderShared(); },
    "note-cancel": function () { state.shared.noteOpen = false; renderShared(); },
    "note-create": function () { createNote(); },
    "note-save": function () { saveNote(); },
    "note-delete": function () { deleteNote(); },

    "reload-preview": function () {
      var frame = el("preview-frame");
      if (frame) frame.src = frame.src;
    },

    "track-path": function () {
      var path = readInput("add-path");
      if (!path) return toast("warn", "Enter a path first");
      act("git.repo.track", { repo: path }).then(function (o) {
        if (o) { state.addOpen = false; navigate("projects"); }
      });
    },

    "init-path": function () {
      var path = readInput("add-path");
      if (!path) return toast("warn", "Enter a path first");
      act("git.repo.init", { path: path }).then(function (o) {
        if (o) {
          return act("git.repo.track", { repo: path }).then(function () { navigate("projects"); });
        }
      });
    },

    "clone-form": function () {
      openDrawer(
        "git.clone",
        '<div class="stack">' +
          '<div class="field"><label class="field-label" for="clone-url">Repository URL</label>' +
          '<input class="input mono" id="clone-url" placeholder="https://host/owner/repo.git"></div>' +
          '<div class="field"><label class="field-label" for="clone-dest">Destination</label>' +
          '<div class="inline"><input class="input mono grow" id="clone-dest" placeholder="Where the clone should go">' +
          '<button type="button" class="btn" data-action="clone-pick">Choose folder…</button></div></div>' +
          '<div class="inline"><button type="button" class="btn btn-primary" data-action="clone-go">Clone</button>' +
          '<span class="faint">Credentials come from your Git credential manager; devd never stores them.</span></div>' +
          "</div>"
      );
    },

    "clone-pick": function () {
      openPicker({
        title: "Choose where to clone into",
        note: "Pick the folder the repository should be cloned into. The repository name is added to it.",
        confirm: "Use this folder",
        intent: "directory",
        start: readInput("clone-dest")
      }).then(function (path) {
        if (!path) return;
        var input = el("clone-dest");
        if (input) input.value = path;
      });
    },

    "clone-go": function () {
      var url = readInput("clone-url");
      var destination = readInput("clone-dest");
      if (!url || !destination) return toast("warn", "A URL and a destination are both required");
      act("git.clone", { url: url, destination: destination }).then(function (o) {
        if (o) { closeDrawer(); navigate("projects"); }
      });
    },

    "open-project": function (node) { openProject(node.dataset.path); },

    "project-forget": function (node) {
      var path = node.dataset.path;
      uiConfirm({
        title: "Remove " + dirName(path) + " from devd?",
        text: "devd forgets the project and any index entry for it. The folder and everything in it are kept, and it can be added again later.",
        confirm: "Remove",
        danger: true
      }).then(function (yes) {
        if (yes) act("project.forget", { path: path });
      });
    },

    "project-refresh": function () {
      return loadProject().then(function () { return loadTab(state.tab); }).then(render);
    },

    untrack: function () {
      var path = project();
      if (!path) return;
      // Removing a project from devd never touches the folder. The words on
      // the button say so, because the difference matters.
      act("project.forget", { path: path }).then(function (o) {
        if (o) navigate("projects");
      });
    },

    "delete-folder": function () {
      var path = project();
      if (!path) return;
      var name = dirName(path);
      openDrawer(
        "Delete " + name + " from disk",
        '<div class="notice notice-bad"><strong>This deletes the folder and everything in it.</strong> ' +
        "The project is removed from devd as well. This cannot be undone.</div>" +
        '<div class="field"><label class="field-label" for="delete-confirm">Type <span class="mono">' +
        esc(name) + "</span> to confirm</label>" +
        '<input class="input mono" id="delete-confirm" autocomplete="off" spellcheck="false"></div>' +
        '<div class="inline"><button type="button" class="btn btn-danger" id="delete-folder-go" data-action="delete-folder-go" disabled>Delete the folder</button>' +
        '<button type="button" class="btn" data-action="close-drawer">Cancel</button></div>'
      );
    },

    "delete-folder-go": function () {
      var path = project();
      var typed = readInput("delete-confirm");
      if (!path) return;
      if (typed !== dirName(path)) {
        toast("error", "The name does not match, so nothing was deleted");
        return;
      }
      closeDrawer();
      act("fs.delete", { path: path, recursive: true }).then(function (o) {
        if (o) {
          return act("project.forget", { path: path });
        }
        return null;
      }).then(function () {
        navigate("projects");
      });
    },

    tab: function (node) {
      state.tab = node.dataset.tab;
      state.diff = null;
      render();
      if (!state.tabs[state.tab] || state.tabs[state.tab].error) {
        loadTab(state.tab).then(render);
      }
    },

    "diff-file": function (node) {
      var path = node.dataset.path;
      var staged = node.dataset.staged === "1";
      read("git.diff", { repo: project(), path: path, staged: staged }).then(function (data) {
        state.diff = { path: path, title: path + (staged ? " (staged)" : ""), patch: data ? data.patch : "" };
        render();
      }).catch(function (e) { toast("error", e.message); });
    },

    stage: function (node) {
      act("git.stage", { repo: project(), paths: [node.dataset.path] });
    },

    unstage: function (node) {
      act("git.unstage", { repo: project(), paths: [node.dataset.path] });
    },

    discard: function (node) {
      act("git.discard", { repo: project(), paths: [node.dataset.path] });
    },

    "stage-all": function () {
      var st = state.project.status;
      var paths = []
        .concat(st.unstaged.map(function (c) { return c.path; }))
        .concat(st.untracked)
        .concat(st.conflicted)
        .concat(st.staged.map(function (c) { return c.path; }));
      if (!paths.length) return toast("warn", "Nothing to stage");
      act("git.stage", { repo: project(), paths: paths });
    },

    "unstage-all": function () {
      var st = state.project.status;
      if (!st.staged.length) return toast("warn", "Nothing is staged");
      act("git.unstage", { repo: project(), paths: st.staged.map(function (c) { return c.path; }) });
    },

    commit: function () {
      var message = readInput("commit-message");
      if (!message) return toast("warn", "Write a commit message first");
      act("git.commit", { repo: project(), message: message, amend: checked("commit-amend") });
    },

    "show-commit": function (node) {
      read("git.show", { repo: project(), rev: node.dataset.rev }).then(function (data) {
        openDrawer("git.show " + node.dataset.rev.slice(0, 8), renderPatch(data ? data.patch : "", 5000));
      }).catch(function (e) { toast("error", e.message); });
    },

    "revert-commit": function (node) {
      act("git.revert", { repo: project(), rev: node.dataset.rev });
    },

    "cherry-pick": function (node) {
      act("git.cherry.pick", { repo: project(), rev: node.dataset.rev });
    },

    "create-branch": function () {
      var name = readInput("new-branch");
      if (!name) return toast("warn", "Name the branch first");
      act("git.branch.create", { repo: project(), name: name, checkout: true });
    },

    switch: function (node) {
      var branch = node.dataset.branch;
      read("git.switch.check", { repo: project(), target: branch })
        .then(function (check) {
          // The check is a real capability query, not a guess: it reports the
          // files the target branch also changes, which are the ones a switch
          // would overwrite.
          if (check && check.safe === false) {
            var files = (check.at_risk || []).join("\n");
            return uiConfirm({ title: "Switch to " + branch + "?", text: "This would overwrite local changes that are not committed, in:\n\n" + files, confirm: "Switch anyway", danger: true }).then(function (go) {
              if (!go) { toast("warn", "Switch to " + branch + " cancelled - local changes preserved"); return null; }
              return act("git.switch", { repo: project(), name: branch });
            });
          }
          return act("git.switch", { repo: project(), name: branch });
        })
        .catch(function (e) {
          toast("error", e.message);
        });
    },

    "merge-current": function (node) {
      act("git.merge", { repo: project(), branch: node.dataset.branch });
    },

    "rename-branch": function (node) {
      uiPrompt({ title: "Rename " + node.dataset.from, input: { value: node.dataset.from, placeholder: "New branch name" }, confirm: "Rename" }).then(function (to) {
        if (!to || to === node.dataset.from) return;
        act("git.branch.rename", { repo: project(), from: node.dataset.from, to: to });
      });
    },

    "delete-branch": function (node) {
      act("git.branch.delete", { repo: project(), name: node.dataset.name });
    },

    "create-tag": function () {
      var name = readInput("new-tag");
      if (!name) return toast("warn", "Name the tag first");
      var message = readInput("new-tag-message");
      act("git.tag.create", { repo: project(), name: name, message: message || null });
    },

    "delete-tag": function (node) {
      act("git.tag.delete", { repo: project(), name: node.dataset.name });
    },

    "push-tags": function () {
      run("git.push", { repo: project(), tags: true, remote: "origin" });
    },

    fetch: function (node) {
      act("git.fetch", { repo: project(), remote: node.dataset.remote || "origin", prune: true });
    },

    pull: function (node) {
      act("git.pull", { repo: project(), remote: node.dataset.remote || "origin" });
    },

    push: function (node) {
      act("git.push", { repo: project(), remote: node.dataset.remote || "origin" });
    },

    "pull-strategy": function () {
      var strategy = readInput("pull-strategy");
      act("git.pull", { repo: project(), remote: "origin", strategy: strategy || null });
    },

    "push-origin": function () {
      act("git.push", { repo: project(), remote: "origin" });
    },

    "push-upstream": function () {
      act("git.push", { repo: project(), remote: "origin", set_upstream: true });
    },

    "force-push": function () {
      act("git.push.force", { repo: project(), remote: "origin" });
    },

    "sync-preview": function () {
      read("git.sync.preview", { repo: project() })
        .then(function (p) {
          var html =
            '<div class="panel"><div class="panel-head"><span>Outgoing</span>' +
            '<span class="panel-note">' + (p.outgoing || []).length + "</span></div>" +
            '<div class="panel-body flush"><div class="rows">' +
            (p.outgoing || []).map(function (c) {
              return '<div class="row"><span class="tag">' + esc(c.short) + "</span>" +
                '<span class="row-main nowrap">' + esc(c.subject) + "</span></div>";
            }).join("") + "</div></div></div>" +
            '<div class="panel"><div class="panel-head"><span>Incoming</span>' +
            '<span class="panel-note">' + (p.incoming || []).length + "</span></div>" +
            '<div class="panel-body flush"><div class="rows">' +
            (p.incoming || []).map(function (c) {
              return '<div class="row"><span class="tag">' + esc(c.short) + "</span>" +
                '<span class="row-main nowrap">' + esc(c.subject) + "</span></div>";
            }).join("") + "</div></div></div>" +
            (p.diverged ? '<div class="notice notice-warn">The branches have diverged.</div>' : "");
          openDrawer("git.sync.preview", html);
        })
        .catch(function (e) { toast("error", e.message); });
    },

    "add-remote": function () {
      var name = readInput("remote-name") || "origin";
      var url = readInput("remote-url");
      if (!url) return toast("warn", "A URL is required");
      act("git.remote.add", { repo: project(), name: name, url: url });
    },

    "remove-remote": function (node) {
      act("git.remote.remove", { repo: project(), name: node.dataset.name });
    },

    "stash-push": function () {
      var message = readInput("stash-message");
      var untracked = checked("stash-untracked");
      act("git.stash.push", { repo: project(), message: message || null, include_untracked: untracked });
    },

    "stash-apply": function (node) {
      act("git.stash.apply", { repo: project(), reference: node.dataset.ref });
    },

    "stash-pop": function (node) {
      act("git.stash.apply", { repo: project(), reference: node.dataset.ref, pop: true });
    },

    "stash-drop": function (node) {
      act("git.stash.drop", { repo: project(), reference: node.dataset.ref });
    },

    "stash-show": function (node) {
      read("git.stash.show", { repo: project(), reference: node.dataset.ref }).then(function (data) {
        openDrawer("git.stash.show " + node.dataset.ref, renderPatch(data ? data.patch : "", 4000));
      }).catch(function (e) { toast("error", e.message); });
    },

    "worktree-add": function () {
      var path = readInput("worktree-path");
      var branch = readInput("worktree-branch");
      if (!path || !branch) return toast("warn", "A path and a branch are both required");
      act("git.worktree.add", {
        repo: project(),
        path: path,
        branch: branch,
        create_branch: checked("worktree-create")
      });
    },

    "worktree-remove": function (node) {
      act("git.worktree.remove", { repo: project(), path: node.dataset.path });
    },

    "worktree-prune": function () {
      act("git.worktree.prune", { repo: project() });
    },

    "conflict-show": function (node) {
      read("git.conflicts.show", { repo: project(), path: node.dataset.path }).then(function (data) {
        openDrawer("git.conflicts.show " + node.dataset.path,
          '<pre class="patch-block">' + esc(data ? data.content : "") + "</pre>");
      }).catch(function (e) { toast("error", e.message); });
    },

    "resolve-ours": function (node) {
      act("git.conflicts.resolve", { repo: project(), path: node.dataset.path, side: "ours" });
    },

    "resolve-theirs": function (node) {
      act("git.conflicts.resolve", { repo: project(), path: node.dataset.path, side: "theirs" });
    },

    "op-continue": function (node) {
      act("git.op.continue", { repo: project(), op: node.dataset.op });
    },

    "op-abort": function (node) {
      act("git.op.abort", { repo: project(), op: node.dataset.op });
    },

    "reset-to": function (node) {
      var mode = readInput("reset-mode") || "mixed";
      act("git.reset", { repo: project(), mode: mode, rev: node.dataset.rev });
    },

    "reset-mode": function () {
      var rev = readInput("recovery-rev");
      var mode = readInput("reset-mode");
      if (!rev) return toast("warn", "Give a revision to reset to");
      act("git.reset", { repo: project(), mode: mode, rev: rev });
    },

    "rebase-onto": function () {
      var rev = readInput("recovery-rev");
      if (!rev) return toast("warn", "Give a revision to rebase onto");
      act("git.rebase", { repo: project(), onto: rev });
    },

    squash: function () {
      var count = Number(readInput("squash-rev"));
      var message = readInput("squash-message");
      if (!count || !message) return toast("warn", "A count and a message are both required");
      act("git.squash", { repo: project(), count: count, message: message });
    },

    "bisect-start": function () {
      var good = readInput("bisect-good");
      var bad = readInput("bisect-bad");
      if (!good || !bad) return toast("warn", "Give both a good and a bad revision");
      act("git.bisect.start", { repo: project(), good: good, bad: bad });
    },

    "bisect-good": function () { act("git.bisect.mark", { repo: project(), verdict: "good" }); },
    "bisect-bad": function () { act("git.bisect.mark", { repo: project(), verdict: "bad" }); },
    "bisect-skip": function () { act("git.bisect.mark", { repo: project(), verdict: "skip" }); },
    "bisect-reset": function () { act("git.bisect.reset", { repo: project() }); },

    "pick-cap": function (node) { openCapability(node.dataset.id); },

    "cap-mode": function (node) {
      var schema = state.selectedCapDetail && state.selectedCapDetail.args_schema;
      var next = node.dataset.mode === "json" ? "json" : "form";
      // Carry what has been typed across: form to JSON reads the controls, JSON to
      // form parses the box. Nothing is lost by changing view.
      if (el("cap-form")) {
        var fromForm = readArgForm(true);
        if (fromForm) state.capArgs = fromForm;
      } else if (el("cap-args")) {
        try { state.capArgs = JSON.parse(readInput("cap-args") || "{}"); }
        catch (e) { toast("error", "The arguments are not valid JSON", e.message); return; }
      }
      state.capMode = next;
      if (schema) renderCapabilities();
    },

    "cap-dry-run": function () {
      var args = parseArgs();
      if (!args) return;
      plan(state.selectedCap, args)
        .then(function (p) {
          // Some capabilities cannot produce a plan. That is a fact about the
          // capability, not an error, and it is worth saying plainly.
          if (!p) {
            openDrawer(
              state.selectedCap + " plan",
              '<div class="notice notice-warn">This capability cannot produce a plan. ' +
              "Running it will act straight away.</div>"
            );
            return;
          }
          openDrawer(
            state.selectedCap + " plan",
            '<div class="notice">' + esc(p.summary) + "</div>" +
              '<ul class="steps">' + (p.steps || []).map(function (s) {
                return "<li>" + esc(s) + "</li>";
              }).join("") + "</ul>" +
              (p.data && typeof p.data === "object"
                ? '<pre class="patch-block">' + esc(JSON.stringify(p.data, null, 2)) + "</pre>"
                : "")
          );
        })
        .catch(function (e) { toast("error", e.message); });
    },

    "cap-run": function () {
      var args = parseArgs();
      if (!args) return;
      run(state.selectedCap, args).then(function (o) {
        if (!o) return;
        openDrawer(state.selectedCap + " result",
          '<pre class="patch-block">' + esc(JSON.stringify(o.data === null ? o : o.data, null, 2)) + "</pre>");
        refresh();
      });
    },

    quit: function () {
      request("/api/shutdown", { method: "POST", body: "{}" }).then(function () {
        document.body.innerHTML =
          '<div class="empty" style="height:100vh;justify-content:center">' +
          '<div class="empty-title">devd stopped</div><div>You can close this window.</div></div>';
      });
    }
  };

  function parseArgs() {
    if (el("cap-form")) return readArgForm(false);
    var text = readInput("cap-args");
    if (!text) return {};
    try {
      return JSON.parse(text);
    } catch (e) {
      toast("error", "The arguments are not valid JSON", e.message);
      return null;
    }
  }

  // --------------------------------------------------------------------- events

  document.addEventListener("click", function (event) {
    if (state.topMenu && !event.target.closest(".top-menu")) { state.topMenu = ""; rail(); }
    var node = event.target.closest("[data-action]");
    if (!node) return;
    var action = node.dataset.action;
    var handler = ACTIONS[action];
    if (handler) {
      event.preventDefault();
      handler(node);
    }
  });

  document.addEventListener("keydown", function (event) {
    var mod = event.ctrlKey || event.metaKey;
    if (mod && event.key.toLowerCase() === "k") {
      event.preventDefault();
      if (state.palette.open) closePalette();
      else openPalette();
      return;
    }
    if (event.key === "Escape") {
      if (state.topMenu) {
        var owner = document.querySelector('.top-tab-menu[aria-expanded="true"]');
        state.topMenu = ""; rail();
        var back = document.querySelector('.top-tab-menu[data-menu]'); if (owner && owner.dataset.menu) { back = document.querySelector('.top-tab-menu[data-menu="' + owner.dataset.menu + '"]'); }
        if (back) back.focus();
        return;
      }
      if (state.palette.open) return closePalette();
      if (picker.open) return closePicker(null);
      if (!el("modal").hidden) return decideModal(false);
      if (!el("drawer").hidden) return closeDrawer();
      return;
    }
    if (state.palette.open && (event.key === "ArrowDown" || event.key === "ArrowUp")) {
      event.preventDefault();
      var items = state.palette.items;
      if (!items.length) return;
      var next = state.palette.index + (event.key === "ArrowDown" ? 1 : -1);
      state.palette.index = (next + items.length) % items.length;
      return renderPalette();
    }
    if (state.palette.open && event.key === "Enter") {
      event.preventDefault();
      return paletteActivate(state.palette.index);
    }
    if (event.key === "Enter" && mod && !el("modal").hidden) {
      event.preventDefault();
      return decideModal(true);
    }
    if (picker.open && event.key === "Enter" && event.target.id === "picker-path-input") {
      event.preventDefault();
      var typed = event.target.value.trim();
      if (typed) pickerGo(typed);
      return;
    }
    if (picker.open && event.key === "Enter" && !event.target.matches("input, textarea")) {
      event.preventDefault();
      var chosen = picker.selected;
      if (chosen) closePicker(chosen);
      return;
    }
    // Keep Tab inside a dialog: focus that escapes behind a modal is the most
    // common keyboard trap in a single-page app.
    if (event.key === "Tab") {
      var dialog = !el("modal").hidden ? el("modal")
        : !el("dialog").hidden ? el("dialog")
        : !el("drawer").hidden ? el("drawer")
        : picker.open ? document.querySelector("#picker .picker")
        : state.palette.open ? document.querySelector("#palette .palette") : null;
      if (!dialog) return;
      var focusables = Array.prototype.filter.call(
        dialog.querySelectorAll('button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'),
        function (node) { return node.offsetParent !== null; }
      );
      if (!focusables.length) return;
      var first = focusables[0], last = focusables[focusables.length - 1];
      if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
      else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
    }
  });

  document.addEventListener("input", function (event) {
    if (event.target.closest && event.target.closest("#cap-form")) {
      var typed = readArgForm(true);
      if (typed) state.capArgs = typed;
      return;
    }
    if (event.target.id === "palette-input") return paletteSearch(event.target.value);
    if (event.target.id === "picker-path-input") {
      // Typing a path is the advanced way in; it still has to work, but it is
      // never the only way.
      picker.selected = event.target.value.trim() || null;
      paintPickerInspect();
      return;
    }
    if (event.target.id === "process-log-query") {
      state.processes.logQuery = event.target.value;
      renderProcesses();
      var input = el("process-log-query");
      if (input) {
        input.focus();
        input.setSelectionRange(input.value.length, input.value.length);
      }
      return;
    }
    if (event.target.id === "delete-confirm") {
      var button = el("delete-folder-go");
      if (button) {
        var path = project();
        button.disabled = !path || event.target.value.trim() !== dirName(path);
      }
      return;
    }
    if (event.target.id === "project-search") {
      state.projectQuery = event.target.value;
      clearTimeout(state.projectSearchTimer);
      state.projectSearchTimer = setTimeout(function () {
        renderProjects();
        var input = el("project-search");
        if (input) {
          input.focus();
          input.setSelectionRange(input.value.length, input.value.length);
        }
      }, 140);
      return;
    }
    if (event.target.id === "cap-search") {
      state.caps.q = event.target.value;
      clearTimeout(state.capSearchTimer);
      state.capSearchTimer = setTimeout(function () {
        loadCapabilities().then(function () {
          // Keep focus and the caret while the list re-renders.
          render();
          var input = el("cap-search");
          if (input) {
            input.focus();
            input.setSelectionRange(input.value.length, input.value.length);
          }
        });
      }, 140);
    }
  });

  document.addEventListener("change", function (event) {
    if (event.target.dataset && event.target.dataset.change && ACTIONS[event.target.dataset.change]) {
      ACTIONS[event.target.dataset.change](event.target);
      return;
    }
    if (event.target.dataset && event.target.dataset.action === "site-php") {
      ACTIONS["site-php"](event.target);
      return;
    }
    if (event.target.id === "preview-site") {
      state.preview.site = event.target.value;
      render();
    }
    if (event.target.id === "process-project") {
      state.processes.project = event.target.value;
      state.processes.plan = null;
      state.processes.status = null;
      state.processes.error = null;
      state.processes.selected = null;
      loadProcesses().then(renderProcesses);
    }
    if (event.target.id === "shared-project") {
      state.shared.project = event.target.value;
      state.shared.activeNote = null;
      state.shared.noteContent = "";
      loadShared().then(renderShared);
    }
    if (event.target.id === "roster-project") {
      state.rosterProject = event.target.value;
      state.rosterCharter = null;
      state.rosterContext = null;
      loadRoster().then(renderRoster);
    }
    if (event.target.dataset && event.target.dataset.action === "todo-status") {
      setTodoStatus(event.target.dataset.id, event.target.dataset.rev, event.target.value);
    }
    if (event.target.id === "studio-project") {
      STUDIO.project = event.target.value;
      STUDIO.selected = null;
      STUDIO.compat = null;
      loadStudio().then(renderStudioTarget);
    }
  });

  el("modal-cancel").addEventListener("click", function () { decideModal(false); });
  el("modal-close").addEventListener("click", function () { decideModal(false); });
  el("drawer-scrim").addEventListener("click", closeDrawer);
  el("modal-confirm").addEventListener("click", function () { decideModal(true); });
  el("picker").addEventListener("click", function (event) {
    if (event.target === el("picker")) closePicker(null);
  });
  el("palette").addEventListener("click", function (event) {
    if (event.target === el("palette")) closePalette();
  });
  el("modal").addEventListener("click", function (event) {
    if (event.target === el("modal")) decideModal(false);
  });


  function startChat(text, project) {
    AI.project = project || "";
    sendChat({ message: text, project: project || undefined });
  }

  function sendChat(args) {
    if (!args.thread) {
      if (AI.pickProvider) args.provider = AI.pickProvider;
      if (AI.pickModel) args.model = AI.pickModel;
    }
    run("chat.send", args, { quiet: true }).then(function (o) {
      if (!o) return;
      var id = (o.data && (o.data.id || o.data.thread)) || args.thread;
      if (!id) return;
      if (o.data && o.data.provider_id) AI.pickProvider = o.data.provider_id;
      if (o.data && o.data.model) AI.pickModel = o.data.model;
      var queued = (o.data && o.data.queued) || 0;
      if (queued > 0 && AI.thread && AI.thread.id === id) {
        // It is working; the header's queue count is the acknowledgement. A
        // toast on every message was noise, not information.
        AI.queued = queued;
        pollThread(false).then(function () { paintChat(); });
        return;
      }
      AI.thread = { id: id, title: "", state: "running", input_tokens: 0, output_tokens: 0 }; AI.lastId = 0; AI.events = []; AI.pending = []; AI.queued = queued;
      state.view = "assistant"; render();
      read("chat.list").then(function (t) { AI.threads = t; return pollThread(true); }).then(renderAssistant);
    }).catch(function (e) {
      // The draft was cleared before the call; put it back so a refusal does not lose it.
      var inp = el("chat-input");
      if (inp && !inp.value) { inp.value = args.message || ""; AI.composer = inp.value; fitComposer(); syncComposerAction(); }
      toast("error", e.message);
    });
  }

  function providerCall(id, cap, extra) {
    var args = { id: id };
    Object.keys(extra || {}).forEach(function (k) { args[k] = extra[k]; });
    AI.provBusy[id] = true; delete AI.provBusy[id + ":msg"]; renderProviders();
    read(cap, args).then(function (d) {
      AI.provBusy[id + ":ok"] = true;
      AI.provBusy[id + ":msg"] = d && d.message ? d.message : (cap === "provider.test" ? "Connection works" : cap === "provider.verify" ? "The model answered" : "Models updated");
    }).catch(function (e) {
      AI.provBusy[id + ":ok"] = false; AI.provBusy[id + ":msg"] = e.message;
    }).then(function () { AI.provBusy[id] = false; return loadProviders(); }).then(function () { renderProviders(); loadHealth().then(healthChip); });
  }

  function setProviderModel(id, model) {
    run("provider.save", { id: id, default_model: model }).then(function (o) { if (o) return loadProviders().then(renderProviders); });
  }

  document.addEventListener("input", function (e) {
    var t = e.target;
    if (t.id === "team-say") { var tsb = el("team-say-send"); if (tsb) tsb.disabled = !t.value.trim(); }
    if (t.id === "home-ask") { AI.askText = t.value; var hs = el("home-send"); if (hs) hs.disabled = !t.value.trim(); }
    else if (t.id === "chat-input") {
      AI.composer = t.value;
      syncComposerAction();
      fitComposer();
      // "@" opens the list of things a message can refer to.
      var trig = mentionTrigger(t.value, t.selectionStart);
      if (!trig) closeMentions();
      else if (!MENTIONS.open || MENTIONS.start !== trig.start || MENTIONS.query !== trig.query) openMentions(trig.start, trig.query);
    } else if (t.dataset && t.dataset.filter) {
      // The list is re-rendered from the same data, so there is nothing to
      // fetch; a short pause keeps the screen from redrawing on every letter.
      state.filters[t.dataset.filter] = t.value;
      clearTimeout(state.filterTimer);
      state.filterTimer = setTimeout(function () {
        render();
        // The box is rebuilt by the render, so the caret has to be put back.
        var again = document.querySelector('[data-filter="' + t.dataset.filter + '"]');
        if (again) { again.focus(); again.setSelectionRange(again.value.length, again.value.length); }
      }, 120);
    }
  });
  /* The drafting box's height depends on the window, so it is measured again when the window changes. */
  window.addEventListener("resize", function () { fitComposer(); });

  document.addEventListener("change", function (e) {
    var t = e.target;
    if (t.id === "home-project") AI.askProject = t.value;
    if (t.id === "chat-project") AI.project = t.value;
    if (t.dataset && t.dataset.chat === "provider") {
      var provider = (AI.providers || []).filter(function (x) { return x.id === t.value; })[0];
      var nextModel = (provider && provider.default_model) || (modelOptions(t.value)[0] || {}).model_id || "";
      AI.pickProvider = t.value;
      AI.pickModel = nextModel;
      if (AI.thread) {
        AI.thread.provider_id = t.value;
        AI.thread.model = nextModel;
        run("chat.update", { thread: AI.thread.id, provider: t.value, model: nextModel || "" }, { quiet: true })
          .then(function () { return pollThread(false); })
          .then(renderAssistant);
      } else {
        renderAssistant();
      }
      return;
    }
    if (t.dataset && t.dataset.chat === "model") {
      AI.pickModel = t.value;
      if (AI.thread) {
        AI.thread.model = t.value;
        run("chat.update", { thread: AI.thread.id, model: t.value }, { quiet: true })
          .then(function () { return pollThread(false); })
          .then(paintChat);
      }
      return;
    }
    if (t.dataset && t.dataset.provModel !== undefined && t.dataset.provModel !== "") setProviderModel(t.dataset.provModel, t.value.trim());
  });
  document.addEventListener("keydown", function (e) {
    if (e.target.id === "team-say" && e.key === "Enter" && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      var tb = el("team-say-send");
      if (tb && !tb.disabled) tb.click();
      return;
    }
    if (e.target.id === "chat-input") {
      // While the list is open the keyboard belongs to the list.
      if (MENTIONS.open) {
        if (e.key === "ArrowDown") { e.preventDefault(); moveMention(1); return; }
        if (e.key === "ArrowUp") { e.preventDefault(); moveMention(-1); return; }
        if (e.key === "Enter" || e.key === "Tab") { e.preventDefault(); pickMention(); return; }
        if (e.key === "Escape") { e.preventDefault(); closeMentions(); return; }
      }
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        var b = el("chat-send"); if (b) b.click();
      }
    }
    if (e.target.id === "home-ask" && e.key === "Enter" && (e.ctrlKey || e.metaKey)) { e.preventDefault(); ACTIONS[AI.homeMode === "team" ? "home-team" : "home-ask"](); }
  });

  document.addEventListener("input", function (e) {
    var t = e.target;
    if (t.id === "team-goal") TEAM.form.goal = t.value;
    if (t.id === "sched-name") SCHED.form.name = t.value;
    if (t.id === "sched-spec") SCHED.form.spec = t.value;
    if (t.id === "sched-params") SCHED.form.params = t.value;
    if (t.id === "sched-goal") SCHED.form.goal = t.value;
    if (t.id === "sched-done") SCHED.form.done = t.value;
    if (t.id === "team-done") TEAM.form.done = t.value;
    if (t.id === "team-name") TEAM.form.name = t.value;
  });
  document.addEventListener("change", function (e) {
    var t = e.target;
    if (t.id === "composer-mode") {
      var hb = el("home-ask"); if (hb) AI.askText = hb.value;
      var cb = el("chat-input"); if (cb) AI.composer = cb.value;
      AI.homeMode = t.value === "team" ? "team" : "ask";
      if (state.view === "home") { renderHome(); var again = el("home-ask"); if (again) again.focus(); }
      else { renderAssistant(); var ci = el("chat-input"); if (ci) ci.focus(); }
      return;
    }
    if (t.id === "chat-file" || t.id === "home-file") {
      var files = Array.prototype.slice.call(t.files || []);
      if (!files.length) return;
      addFiles(files).then(function () { t.value = ""; });
      return;
    }
    if (t.id === "team-template") {
      TEAM.form.template = t.value;
      if (!t.value) return;
      read("template.get", { id: t.value }).then(function (tpl) {
        TEAM.form.goal = tpl.body;
        TEAM.form.done = (tpl.done_when || []).join("\n");
        paintTeam();
        toast("info", "Replace the {{blanks}} in the goal with your own words before you start.");
      });
      return;
    }
    if (t.id === "sched-project") SCHED.form.project = t.value;
    if (t.id === "sched-template") { SCHED.form.template = t.value; renderSchedules(); return; }
    if (t.id === "sched-waive") SCHED.form.waive = t.checked;
    if (t.id === "team-project") { TEAM.form.project = t.value; paintTeam(); }
    if (t.id === "team-parallel") TEAM.form.parallel = parseInt(t.value, 10);
  });
  // ----------------------------------------------------------------------- boot

  /* Repaint only the parts around the page: the rail count, the tabs, the
     breadcrumb and the status bar. Background work uses this so a slow probe
     cannot rebuild a screen while somebody is typing in it. */
  function paintChrome() {
    rail();
    subnav();
    crumbs();
    statusbar();
    healthChip();
  }

  function boot() {
    render();
    // The address is read first: a link to a screen, a bookmark or the back
    // button all say where the person wants to be.
    readRoute();
    // Session first, then only the data the current screen needs. The rest of
    // the environment is read beside it and repaints the chrome as it arrives.
    // A doctor run or a health probe must never hold the first screen hostage.
    return loadSession()
      .then(function () {
        var current = state.view;
        var first = refresh();
        if (current !== "home" && current !== "projects" && current !== "project") {
          loadProjects().then(paintChrome).catch(function () {});
        }
        if (SERVER_VIEWS.indexOf(current) === -1 && current !== "home") {
          loadServer().then(paintChrome).catch(function () {});
        }
        return first.then(function () { render(); });
      })
      .then(function () {
        if (state.session && state.session.pending_approvals) {
          toast("warn", state.session.pending_approvals + " operations are awaiting a decision");
        }
        // The health chip is useful on every screen, but it is not a gate.
        loadHealth().then(function () {
          paintChrome();
          if (state.view === "home") render();
        }).catch(function () {});
        loadDoctor().then(function () {
          if (state.view === "system" || state.view === "home") render();
        }).catch(function () {});
      })
      .then(function () {
        // A conversation named in the address is opened once its list is in.
        var id = state.pendingThread;
        if (!id) return;
        state.pendingThread = null;
        var t = (AI.threads || []).filter(function (x) { return x.id === id; })[0];
        if (t) ACTIONS["open-thread"]({ dataset: { id: id } });
        else navigate("assistant", true);
      });
  }

  window.addEventListener("popstate", function () { readRoute(); });

  // Dropping a file on the composer is the same act as choosing it in the
  // picker. The upload path is what the message carries, so one path handles
  // both inputs.
  document.addEventListener("dragover", function (event) {
    if (event.target.closest && event.target.closest(".composer")) event.preventDefault();
  });
  document.addEventListener("drop", function (event) {
    var composer = event.target.closest && event.target.closest(".composer");
    if (!composer) return;
    event.preventDefault();
    var files = Array.prototype.slice.call((event.dataTransfer && event.dataTransfer.files) || []);
    if (!files.length) return;
    addFiles(files);
  });


  document.addEventListener("input", onCreateInput);
  document.addEventListener("change", onCreateInput);
  function onCreateInput(event) {
    var t = event.target;
    if (!t.dataset || t.dataset.create === undefined) return;
    if (event.type === "input" && (t.type === "checkbox" || t.tagName === "SELECT")) return;
    var key = t.dataset.create, f = CREATE.form;
    if (key === "__advanced") { CREATE.advanced = t.checked; renderCreate(); return; }
    var val = t.type === "checkbox" ? t.checked : t.value;
    if (t.dataset.multi !== undefined) {
      var cur = (f.options[key] || []).slice(), i = cur.indexOf(t.dataset.multi);
      if (t.checked && i < 0) cur.push(t.dataset.multi);
      if (!t.checked && i >= 0) cur.splice(i, 1);
      f.options[key] = cur;
    } else if (t.dataset.opt) f.options[key] = val;
    else f[key] = val;
    scheduleValidate();
    if (event.type === "change" && (key === "version" || key === "package_manager")) { /* values only; review repaints */ }
  }

  setInterval(function () {
    if (document.hidden) return;
    loadHealth().then(function () { healthChip(); rail(); statusbar(); });
  }, 30000);

  // Work in flight is shown in the command bar on every screen, so the runs are re-read on a
  // calm cadence from anywhere (every read is also written to Activity, so not faster than needed); the team screen also keeps its own faster timer for its detail.
  // Approvals and questions are counted in the same breath, so "Needs you" never lags behind
  // a decision that was just made.
  // A run in flight is read every 4 seconds, since that is when a change is worth seeing
  // quickly; with nothing running it is every 10.
  function pollLive() {
    if (document.hidden) return setTimeout(pollLive, 10000);
    Promise.all([read("team.list").catch(function () { return null; }), read("approval.list", { pending_only: true }).catch(function () { return null; })]).then(function (r) {
      var changed = false;
      if (Array.isArray(r[0])) { var sig = r[0].map(function (t) { return t.id + t.state; }).join("|"); if (sig !== AI.liveSig) { AI.liveSig = sig; AI.liveRuns = r[0]; changed = true; } }
      if (Array.isArray(r[1]) && r[1].length !== AI.counts.approvals) { AI.counts.approvals = r[1].length; changed = true; }
      if (changed) { rail(); statusbar(); }
    }).then(function () {
      var busy = (AI.liveRuns || []).some(function (t) { return t.state === "planning" || t.state === "running"; });
      setTimeout(pollLive, busy ? 4000 : 10000);
    });
  }
  setTimeout(pollLive, 10000);

  // ------------------------------------------------------------------ live sync
  //
  // The projects, folders and files on this machine are changed by other programs too: an editor
  // saves a file, a package manager creates a folder, a build removes a lock file. The screen is
  // re-read on a cadence, and the markup guard below repaints only when the answer actually changed,
  // so an unchanged machine costs nothing and a changed one appears without a reload. Nothing is
  // re-read while a person is typing, dragging a selection, or has a sheet open: pulling the page
  // from under them is worse than a five-second lag.

  var LIVE = { busy: false, last: 0 };
  function liveIsEditing() {
    var a = document.activeElement;
    if (a && (a.tagName === "INPUT" || a.tagName === "TEXTAREA" || a.tagName === "SELECT" || a.isContentEditable)) return true;
    if (state.palette && state.palette.open) return true;
    if (document.querySelector(".drawer, .modal, [role=\"dialog\"]")) return true;
    var sel = window.getSelection ? window.getSelection() : null;
    if (sel && sel.rangeCount && !sel.isCollapsed) return true;
    return false;
  }
  function liveTick() {
    if (document.hidden || LIVE.busy || state.busy) return;
    var every = { projects: 15000, project: 7000, sites: 5000, site: 6000, inbox: 5000, mcp: 10000, home: 20000 }[state.view];
    if (!every) return;
    var now = Date.now();
    if (now - LIVE.last < every || liveIsEditing()) return;
    var load;
    if (state.view === "projects") load = function () { return loadProjects(true); };
    else if (state.view === "sites" || state.view === "site") load = loadServer;
    else if (state.view === "inbox") load = loadInbox;
    else if (state.view === "mcp") load = loadMcp;
    else if (state.view === "home") load = loadHome;
    else if (state.view === "project") load = function () { return loadProject().then(function () { return loadTab(state.tab); }); };
    if (!load) return;
    var view = state.view;
    LIVE.last = now;
    LIVE.busy = true;
    load().then(function () { if (state.view === view) render(); }).catch(function () { /* the next tick tries again */ }).then(function () { LIVE.busy = false; });
  }
  setInterval(liveTick, 1000);

  // Views that repaint a panel in place (the assistant poll, provider actions)
  // bypass render(), so a small observer keeps their tables scrollable too.
  var mainNode = el("main");
  if (mainNode && window.MutationObserver) {
    new MutationObserver(function () { wrapTables(); }).observe(mainNode, { childList: true, subtree: true });
  }

  // Opening a tab paints the page, then paints it again when its data arrives; the second paint
  // is usually the same page. Setting identical markup would rebuild every element (and reload
  // the preview's frame), which shows as a flicker, so on these pages an unchanged page is left alone.
  (function () {
    var node = el("main");
    var desc = Object.getOwnPropertyDescriptor(Element.prototype, "innerHTML");
    var calm = ["projects", "home", "inbox", "mcp", "project", "overview", "services", "sites", "site", "operations", "toolchain", "runtimes", "databases", "remote", "preview", "certificates", "hosts"];
    var last = null;
    if (!node || !desc) return;
    Object.defineProperty(node, "innerHTML", {
      configurable: true,
      get: function () { return desc.get.call(this); },
      set: function (html) {
        if (calm.indexOf(state.view) >= 0 && html === last) return;
        // A live refresh may replace a page the person is reading; keep their place when it does.
        if (calm.indexOf(state.view) >= 0) {
          var scroller = document.scrollingElement || document.documentElement;
          var top = scroller ? scroller.scrollTop : 0;
          last = html;
          desc.set.call(this, html);
          if (scroller) scroller.scrollTop = top;
          return;
        }
        last = html;
        desc.set.call(this, html);
      }
    });
  })();

  // ----------------------------------------------------------------- actions

  function opsLines(text) { return String(text || "").split(/\r?\n/); }

  Object.assign(ACTIONS, {
    "ops-tab": function (n) {
      var tab = n.dataset.tab;
      if (state.view === "site") SITE.tab = tab;
      else OPS.tab = tab;
      OPS.error = "";
      OPS.errorTab = "";
      render();
      opsLoad(tab);
    },
    "site-tab": function (n) {
      SITE.tab = n.dataset.tab;
      OPS.error = "";
      OPS.errorTab = "";
      render();
    },
    "remote-tab": function (n) {
      REMOTE.tab = n.dataset.tab;
      OPS.error = "";
      OPS.errorTab = "";
      render();
    },
    "open-site": function (n) { openSite(n.dataset.name); },
    "verify-site": function (n) { verifySite(n.dataset.name); },
    "verify-sites": function () {
      var names = opsSiteNames();
      // One at a time: each check opens one connection, and the list fills in as it goes.
      names.reduce(function (p, name) { return p.then(function () { return verifySite(name, true); }); }, Promise.resolve())
        .then(function () {
          var bad = names.filter(function (x) { return SITE_VERIFY[x] && SITE_VERIFY[x].report && !SITE_VERIFY[x].report.ok; }).length;
          toast(bad ? "warn" : "ok", bad ? bad + " of " + names.length + " sites did not answer" : "All " + names.length + " sites answered");
        });
    },
    "ops-site": function (n) { OPS.site = n.value; OPS.data = {}; OPS.error = ""; OPS.errorTab = ""; render(); opsLoad(OPS.tab); },
    "ops-level": function (n) {
      var l = n.dataset.level, a = OPS.levels || [];
      OPS.levels = a.indexOf(l) >= 0 ? a.filter(function (x) { return x !== l; }) : a.concat([l]);
      opsLoad("logs");
    },
    "ops-logkind": function (n) { OPS.logKind = n.value; opsLoad("logs"); },
    "ops-log-clear": function () {
      uiConfirm({ title: "Empty the Laravel log?", text: "A copy of each file is kept as .log.bak.", confirm: "Empty", danger: true }).then(function (ok) {
        if (ok) opsAct("logs.laravel_clear", { site: OPS.site });
      });
    },
    "ops-env-set": function (n) {
      var name = n.dataset.name;
      uiPrompt({ title: name ? "Change " + name : "Add a variable", text: name ? "The new value." : "Write it as NAME=value.", confirm: "Save", input: { placeholder: name ? "value" : "APP_ENV=local" } }).then(function (v) {
        if (v === null || v === "") return;
        var k = name, val = v;
        if (!name) { var i = v.indexOf("="); if (i < 1) return toast("error", "write it as NAME=value"); k = v.slice(0, i).trim(); val = v.slice(i + 1); }
        var vars = {}; vars[k] = val;
        opsAct("env.set", { site: OPS.site, vars: vars });
      });
    },
    "ops-env-unset": function (n) {
      uiConfirm({ title: "Remove " + n.dataset.name + "?", text: "The old file is kept as a backup beside it.", confirm: "Remove", danger: true }).then(function (ok) {
        if (ok) opsAct("env.unset", { site: OPS.site, names: [n.dataset.name] });
      });
    },
    "ops-env-reveal": function () {
      read("env.reveal", { site: OPS.site }).then(function (r) {
        var lines = Object.keys(r.vars || {}).map(function (k) { return k + "=" + r.vars[k]; });
        uiSheet(".env of " + OPS.site, '<pre class="log-block">' + esc(lines.join("\n")) + "</pre>");
      }).catch(function (e) { toast("error", e.message); });
    },
    "ops-env-diff": function () {
      uiPrompt({ title: "Compare the .env", text: "Type a file to compare with (a copy of the production .env), or leave empty to compare with the linked Ploi site.", confirm: "Compare", input: { placeholder: "C:\\path\\to\\production.env" } }).then(function (f) {
        if (f === null) return;
        read("env.diff", f ? { site: OPS.site, against_file: f } : { site: OPS.site, against_ploi: true }).then(function (r) {
          var d = r.diff || {};
          var html = "";
          (d.warnings || []).forEach(function (w) { html += '<div class="notice notice-warn">' + esc(w) + "</div>"; });
          function list(title, names) { return names && names.length ? "<h3>" + esc(title) + " (" + names.length + ")</h3><p class=\"mono\">" + names.map(esc).join("<br>") + "</p>" : ""; }
          html += list("Only here", d.only_here) + list("Only in " + r.against, d.only_there);
          if ((d.different || []).length) html += "<h3>Different (" + d.different.length + ')</h3><p class="mono">' + d.different.map(function (c) { return esc(c.name) + ": " + esc(c.here) + " → " + esc(c.there); }).join("<br>") + "</p>";
          html += '<p class="dim">' + d.same + " variables are the same.</p>";
          uiSheet("Compared with " + r.against, html);
        }).catch(function (e) { toast("error", e.message); });
      });
    },
    "ops-cron-add": function () {
      uiPrompt({ title: "When should it run?", text: "Five fields: minute hour day-of-month month day-of-week. Or @hourly, @daily.", confirm: "Next", input: { placeholder: "*/5 * * * *" } }).then(function (sch) {
        if (!sch) return;
        uiPrompt({ title: "What should it run?", text: "One command, run in the site's folder with its PHP.", confirm: "Add job", input: { placeholder: "php artisan reports:send" } }).then(function (cmd) {
          if (cmd) opsAct("cron.add", { site: OPS.site, schedule: sch, command: cmd });
        });
      });
    },
    "ops-cron-laravel": function () { opsAct("cron.laravel", { site: OPS.site }); },
    "ops-cron-run": function (n) {
      run("cron.run", { id: n.dataset.id }).then(function (o) {
        if (o && o.data) uiSheet("Output", '<pre class="log-block">' + esc(o.data.log_tail || "") + "</pre>");
        opsLoad("cron");
      });
    },
    "ops-cron-log": function (n) {
      uiSheet("Cron log", '<div class="dim">Loading…</div>');
      read("cron.log", { id: n.dataset.id, lines: 200 }).then(function (r) { uiSheet("Cron log", '<pre class="log-block">' + esc((r.lines || []).join("\n") || "The job has not printed anything.") + "</pre>"); }).catch(function (e) { uiSheet("Cron log", '<div class="notice notice-bad">' + esc(e.message) + "</div>"); });
    },
    "ops-cron-toggle": function (n) { opsAct("cron.enable", { id: n.dataset.id, enabled: n.dataset.on === "1" }); },
    "ops-cron-remove": function (n) {
      uiConfirm({ title: "Remove this job?", confirm: "Remove", danger: true }).then(function (ok) { if (ok) opsAct("cron.remove", { id: n.dataset.id }); });
    },
    "ops-queue-add": function () {
      uiPrompt({ title: "Add a queue worker", text: "How many processes? (1 to 8)", confirm: "Add", input: { value: "1" } }).then(function (p) {
        if (p) opsAct("queue.add", { site: OPS.site, processes: Math.max(1, Math.min(8, parseInt(p, 10) || 1)) });
      });
    },
    "ops-queue-add-command": function () {
      uiPrompt({ title: "Run a program in the background", text: "A command to run in the site's folder; devd restarts it if it stops.", confirm: "Add", input: { placeholder: "npm run dev" } }).then(function (c) {
        if (c) opsAct("queue.add", { site: OPS.site, command: c });
      });
    },
    "ops-queue": function (n) { opsAct("queue." + n.dataset["do"], { id: n.dataset.id }); },
    "ops-queue-remove": function (n) {
      uiConfirm({ title: "Remove this worker?", text: "It is stopped first; a running job is cut off.", confirm: "Remove", danger: true }).then(function (ok) { if (ok) opsAct("queue.remove", { id: n.dataset.id }); });
    },
    "ops-queue-log": function (n) {
      uiSheet("Worker log", '<div class="dim">Loading…</div>');
      read("queue.logs", { id: n.dataset.id, lines: 200 }).then(function (r) { uiSheet("Worker log", '<pre class="log-block">' + esc((r.lines || []).join("\n") || "The log is empty.") + "</pre>"); }).catch(function (e) { uiSheet("Worker log", '<div class="notice notice-bad">' + esc(e.message) + "</div>"); });
    },
    "ops-redirect-add": function () {
      uiPrompt({ title: "Add a redirect", text: "Write the old path, a space, then where it goes. /blog/* covers a folder, and $1 is what the star matched.", confirm: "Add", input: { placeholder: "/old /new" } }).then(function (v) {
        if (!v) return;
        var parts = v.trim().split(/\s+/);
        if (parts.length < 2) return toast("error", "write the old path, a space, then the new one");
        opsAct("redirects.add", { site: OPS.site, from: parts[0], to: parts[1], status: parts[2] ? parseInt(parts[2], 10) : 301 });
      });
    },
    "ops-redirect-remove": function (n) { opsAct("redirects.remove", { id: n.dataset.id }); },
    "ops-deploy": function () {
      run("deploy.run", { site: OPS.site }).then(function (o) {
        opsLoad("deploy");
        if (o) setTimeout(function () { opsLoad("deploy"); }, 4000);
      });
    },
    "ops-script": function () {
      var d = OPS.data.deploy && OPS.data.deploy[1];
      uiSheet("Deploy script", '<p class="dim">One command per line. {BRANCH} and {SITE_DIRECTORY} are filled in. A failing line stops the deploy. Lines for a server (cd, sudo, PHP-FPM reloads) are skipped here.</p>' +
        '<textarea id="ops-script-text" class="textarea mono" rows="14" spellcheck="false">' + esc((d && d.script) || "") + '</textarea>' +
        '<div class="inline" style="margin-top:12px"><button type="button" class="btn btn-primary" data-action="ops-script-save">Save script</button></div>');
    },
    "ops-script-save": function () {
      var t = el("ops-script-text");
      if (!t) return;
      opsAct("deploy.script_set", { site: OPS.site, script: t.value }, function () { closeDrawer(); });
    },
    "ops-deploy-log": function (n) {
      uiSheet("Deployment output", '<div class="dim">Loading…</div>');
      read("deploy.log", { id: n.dataset.id, lines: 300 }).then(function (r) { uiSheet("Deployment output", '<pre class="log-block">' + esc((r.lines || []).join("\n")) + "</pre>"); }).catch(function (e) { uiSheet("Deployment output", '<div class="notice notice-bad">' + esc(e.message) + "</div>"); });
    },
    "ops-rollback": function (n) {
      uiConfirm({ title: "Roll back to this deployment?", text: "Checks out the code it ended on and runs the script without pulling. Refused while there are uncommitted changes.", confirm: "Roll back", danger: true }).then(function (ok) {
        if (ok) opsAct("deploy.rollback", { site: OPS.site, deployment: n.dataset.id });
      });
    },
    "ops-repo-link": function () {
      uiPrompt({ title: "Branch to deploy", text: "The branch deploys pull. Leave empty for the current one.", confirm: "Link", input: { placeholder: "main" } }).then(function (b) {
        if (b !== null) opsAct("repo.link", { site: OPS.site, branch: b || null });
      });
    },
    "ops-hook": function (n) {
      var on = n.dataset.on === "1";
      run("deploy.hook", { site: OPS.site, enabled: on }).then(function (o) {
        if (o && on && o.data && o.data.path) uiSheet("Push-to-deploy address", '<p>POST to this path on the devd server to deploy. It is a secret.</p><pre class="log-block">' + esc(o.data.path) + '</pre><p class="dim">' + esc(o.data.note) + "</p>");
        opsLoad("deploy");
      });
    },
    "ops-notify-add": function () {
      uiPrompt({ title: "Where should alerts go?", text: "Type inbox, or the https address of a Slack, Discord or other webhook.", confirm: "Add", input: { placeholder: "inbox" } }).then(function (v) {
        if (!v) return;
        var channel = "inbox", url = null;
        if (v !== "inbox") { url = v; channel = /discord/i.test(v) ? "discord" : /slack/i.test(v) ? "slack" : "webhook"; }
        notifyAct("notify.add", { channel: channel, url: url });
      });
    },
    "ops-notify-test": function (n) { run("notify.test", { id: n.dataset.id }); },
    "ops-notify-remove": function (n) { notifyAct("notify.remove", { id: n.dataset.id }); },
    "ops-php-profile": function (n) { opsAct("php.configure", { profile: n.value }); },
    "ops-php-workers": function (n) { opsAct("php.configure", { workers: parseInt(n.value, 10) }); },
    "ops-site-profile": function (n) { opsAct("php.site_profile", { site: n.dataset.site, profile: n.value || null }); },
    "ops-site-profile-go": function (n) { opsAct("php.site_profile", { site: OPS.site, profile: n.dataset.profile }); },
    "ops-limits": function () {
      uiPrompt({ title: "What does your server allow?", text: "Read them from the PHP tab of your server in Ploi. Write them as name=value, separated by commas: memory_limit=128M, upload_max_filesize=5M, post_max_size=8M, max_execution_time=30.", confirm: "Save", input: { placeholder: "memory_limit=128M, upload_max_filesize=5M" } }).then(function (v) {
        if (!v) return;
        var args = {};
        v.split(",").forEach(function (part) { var i = part.indexOf("="); if (i > 0) args[part.slice(0, i).trim()] = part.slice(i + 1).trim(); });
        if (!Object.keys(args).length) return toast("error", "write them as name=value");
        opsAct("ploi.limits_set", args);
      });
    },
    "ops-php-value": function (n) {
      uiPrompt({ title: "Set a PHP value for " + n.dataset.series, text: "Write it as name=value, e.g. memory_limit=512M.", confirm: "Save", input: { placeholder: "memory_limit=512M" } }).then(function (v) {
        if (!v) return;
        var i = v.indexOf("=");
        if (i < 1) return toast("error", "write it as name=value");
        var vals = {}; vals[v.slice(0, i).trim()] = v.slice(i + 1).trim();
        opsAct("php.configure", { values: vals, series: n.dataset.series });
      });
    },
    "ops-ext": function (n) { opsAct("php.extension_set", { name: n.dataset.name, enabled: n.dataset.on === "1" }); },
    "ops-ploi-connect": function () {
      uiPrompt({ title: "Connect Ploi", text: "Paste an API token from ploi.io/profile/api-keys. It is stored privately on this computer and never shown again.", confirm: "Connect", input: { placeholder: "API token" } }).then(function (t) {
        if (t) opsAct("ploi.connect", { api_token: t });
      });
    },
    "ops-ploi-disconnect": function () {
      uiConfirm({ title: "Disconnect Ploi?", text: "The stored token is deleted. It still exists in Ploi until you delete it there.", confirm: "Disconnect", danger: true }).then(function (ok) { if (ok) { OPS.ploiSites = null; opsAct("ploi.disconnect", {}); } });
    },
    "ops-ploi-sites": function (n) {
      OPS.ploiServer = n.dataset.server;
      read("ploi.sites", { server: parseInt(n.dataset.server, 10) }).then(function (r) { OPS.ploiSites = r.sites || []; render(); }).catch(function (e) { toast("error", e.message); });
    },
    "ops-ploi-link": function (n) {
      uiPrompt({ title: "Link to a local site", text: "Which local site is this? One of: " + opsSiteNames().join(", "), confirm: "Link", input: { value: OPS.site } }).then(function (local) {
        if (local) opsAct("ploi.link", { local: local, server: parseInt(n.dataset.server, 10), site: parseInt(n.dataset.site, 10) });
      });
    },
    "ops-ploi-deploy": function (n) {
      uiConfirm({ title: "Deploy " + n.dataset.domain + " on Ploi?", text: "This starts a deployment on your live server.", confirm: "Deploy", danger: true }).then(function (ok) {
        if (ok) run("ploi.deploy", { server: parseInt(n.dataset.server, 10), site: parseInt(n.dataset.site, 10) });
      });
    }
  });


  Object.assign(ACTIONS, {
    "php-pool": function (n) { act("php.pool", { series: n.dataset.series, action: n.dataset["do"] }); },
    "setup-open": function () { uiSheet("Set up devd", setupForm()); },
    "setup-go": function () { setupStart(); }
  });

  // ------------------------------------------------ operations: actions for the tabs added after the first nine

  function opsWords(text) { return String(text || "").trim().split(/\s+/).filter(Boolean); }

  Object.assign(ACTIONS, {
    // Laravel
    "ops-lv-down": function () {
      uiPrompt({ title: "Maintenance mode", text: "Optionally a secret path that still reaches the site while it is down, e.g. let-me-in. Leave empty for none.", confirm: "Put it down", input: { placeholder: "secret (optional)" } }).then(function (secret) {
        if (secret === null) return;
        opsAct("laravel.down", { site: OPS.site, secret: secret || null });
      });
    },
    "ops-lv-up": function () { opsAct("laravel.up", { site: OPS.site }); },
    "ops-lv-storage": function () { opsAct("laravel.storage_link", { site: OPS.site }); },
    "ops-lv-clear": function () {
      uiPrompt({ title: "Clear caches", text: "What: all (views, configuration, routes, events), or any of views, config, routes, events, cache. The application cache empties what the cache driver holds.", confirm: "Clear", input: { value: "all" } }).then(function (w) {
        if (w === null) return;
        opsAct("laravel.clear", { site: OPS.site, what: opsWords(w.replace(/,/g, " ")) });
      });
    },
    "ops-lv-artisan": function () {
      uiPrompt({ title: "Run an artisan command", text: "Written as after `php artisan`, e.g. migrate:status. It runs with this site's PHP in its folder.", confirm: "Run", input: { placeholder: "migrate:status" } }).then(function (c) {
        if (!c) return;
        run("laravel.artisan", { site: OPS.site, command: c }).then(function (o) {
          if (o && o.data) opsSheetText("artisan " + o.data.command + " (exit " + o.data.exit + ")", o.data.output || "(no output)");
          opsLoad("laravel");
        });
      });
    },
    // databases
    "ops-db-user-add": function () {
      uiPrompt({ title: "Add a database user", text: "A name, then optionally a database it gets every privilege on: shop_app shop", confirm: "Create", input: { placeholder: "name [database]" } }).then(function (v) {
        var w = opsWords(v);
        if (!w.length) return;
        run("db.user_create", { name: w[0], database: w[1] || null }).then(function (o) {
          if (o && o.data) uiSheet("Database user " + o.data.user, "<p>" + esc(o.data.note) + "</p>" + (o.data.password ? '<pre class="log-block">' + esc("user: " + o.data.user + "\npassword: " + o.data.password) + "</pre>" : ""));
          opsLoad("db");
        });
      });
    },
    "ops-db-user-drop": function (n) {
      uiConfirm({ title: "Drop " + n.dataset.user + "?", text: "Apps that connect as this user stop working. Its databases are not touched.", confirm: "Drop", danger: true }).then(function (ok) { if (ok) opsAct("db.user_drop", { name: n.dataset.user, host: n.dataset.host }); });
    },
    "ops-backup-add": function () {
      uiPrompt({ title: "Back up a database", text: "The database, when (cron, or @daily), and optionally how many to keep: shop @daily 7", confirm: "Schedule", input: { placeholder: "shop @daily 7" } }).then(function (v) {
        var w = opsWords(v);
        if (w.length < 2) return;
        var keep = /^\d+$/.test(w[w.length - 1]) && w.length > 2 ? parseInt(w.pop(), 10) : null;
        opsAct("backup.add", { database: w[0], schedule: w.slice(1).join(" "), keep: keep });
      });
    },
    "ops-backup-run": function (n) { opsAct("backup.run", { id: n.dataset.id }); },
    "ops-backup-remove": function (n) {
      uiConfirm({ title: "Stop this backup schedule?", text: "The files already taken stay.", confirm: "Stop", danger: true }).then(function (ok) { if (ok) opsAct("backup.remove", { id: n.dataset.id }); });
    },
    "ops-backup-files": function (n) {
      read("backup.files", { database: n.dataset.db }).then(function (r) {
        var f = r.files || [];
        uiSheet("Backups of " + n.dataset.db, f.length ? '<pre class="log-block">' + esc(f.map(function (x) { return x.name + "  " + Math.round(x.bytes / 1024) + " KB\n" + x.file; }).join("\n\n")) + '</pre><p class="dim">Restore one by giving its path to db.restore.</p>' : '<p class="dim">No backup has been taken yet.</p>');
      }).catch(function (e) { toast("error", e.message); });
    },
    // nginx
    "ops-nx-edit": function () {
      var cur = (OPS.data.nginx && OPS.data.nginx[0] && OPS.data.nginx[0].directives) || "";
      uiSheet("nginx directives for " + OPS.site, '<p class="dim">Written as in an nginx config, inside this site\'s server block. They are checked with nginx -t and put back if nginx rejects them.</p><textarea id="ops-nx-text" class="textarea mono" rows="12" spellcheck="false">' + esc(cur) + '</textarea><div class="inline" style="margin-top:12px"><button type="button" class="btn btn-primary" data-action="ops-nx-save">Save</button></div>');
    },
    "ops-nx-save": function () {
      var t = el("ops-nx-text"); if (!t) return;
      opsAct("nginx.site_set", { site: OPS.site, directives: t.value }, function () { closeDrawer(); });
    },
    "ops-nx-preset": function (n) { opsAct("nginx.site_set", { site: OPS.site, presets: [n.dataset.name] }); },
    "ops-nx-clear": function () {
      uiConfirm({ title: "Remove the custom directives?", text: "The site goes back to the generated configuration only.", confirm: "Remove", danger: true }).then(function (ok) { if (ok) opsAct("nginx.site_clear", { site: OPS.site }); });
    },
    "ops-nx-generated": function () {
      var g = (OPS.data.nginx && OPS.data.nginx[0] && OPS.data.nginx[0].generated) || [];
      opsSheetText("Generated for " + OPS.site, g.join("\n\n") || "No server block was found; start the web server once.");
    },
    // health
    "ops-health-add": function () {
      uiPrompt({ title: "Watch a site", text: "The site, and optionally an address to ask instead of devd's own web server (your public URL).", confirm: "Watch", input: { value: OPS.site } }).then(function (v) {
        var w = opsWords(v);
        if (!w.length) return;
        opsAct("health.add", { site: w[0], url: w[1] || null });
      });
    },
    "ops-health-check": function (n) { opsAct("health.check", { site: n.dataset.site }); },
    "ops-health-remove": function (n) { opsAct("health.remove", { site: n.dataset.site }); },
    // servers
    "ops-srv-add": function () {
      uiPrompt({ title: "Add a server", text: "name host user key [provider] [port], e.g. hetzner-prod 203.0.113.7 deploy id_ed25519 hetzner. The key is a name from the SSH keys tab; provider is a label (digitalocean, hetzner, vultr, linode, aws, ploi, forge, other).", confirm: "Add", input: { placeholder: "name host user key" } }).then(function (v) {
        var w = opsWords(v);
        if (w.length < 4) return toast("error", "write at least: name host user key");
        var port = w.length > 5 ? parseInt(w[5], 10) : null;
        opsAct("server.add", { name: w[0], host: w[1], user: w[2], key: w[3], provider: w[4] || null, port: port });
      });
    },
    "ops-srv-connect": function (n) {
      run("server.connect", { server: n.dataset.name }).then(function (o) {
        if (o && o.data) {
          var s = o.data.server || {};
          uiSheet("Connected to " + n.dataset.name, "<p>" + esc(o.data.os || o.data.kernel || "") + " · logged in as " + esc(o.data.login_user || "") + "</p>" + (s.fingerprint ? '<p class="dim">The host key devd pinned (compare it with your provider\'s console):</p><pre class="log-block">' + esc(s.fingerprint) + "</pre>" : ""));
        }
        opsLoad("servers");
      });
    },
    "ops-srv-info": function (n) {
      run("server.info", { server: n.dataset.name }).then(function (o) {
        if (o && o.data) opsSheetText(n.dataset.name, Object.keys(o.data.info || {}).map(function (k) { return k + ": " + o.data.info[k]; }).join("\n"));
      });
    },
    "ops-srv-provision": function (n) {
      uiPrompt({ title: "Set " + n.dataset.name + " up for PHP", text: "PHP version, database (mariadb, mysql or none), Redis (yes or no). Installs nginx, PHP-FPM, Composer, Node, the database, supervisor and certbot, and turns the firewall on with SSH, HTTP and HTTPS open. Ubuntu or Debian, as root or with passwordless sudo.", confirm: "Install", input: { value: "8.4 mariadb no" } }).then(function (v) {
        if (v === null) return;
        var w = opsWords(v);
        run("server.provision", { server: n.dataset.name, php: w[0] || null, database: w[1] || null, redis: /^y/i.test(w[2] || "") });
      });
    },
    "ops-srv-run": function (n) {
      uiPrompt({ title: "Run on " + n.dataset.name, text: "One command line, run as the server's SSH user.", confirm: "Run", input: { placeholder: "df -h" } }).then(function (c) {
        if (!c) return;
        run("server.run", { server: n.dataset.name, command: c }).then(function (o) { if (o && o.data) opsSheetText("exit " + o.data.code, (o.data.stdout || "") + (o.data.stderr || "")); });
      });
    },
    "ops-srv-remove": function (n) {
      uiConfirm({ title: "Forget " + n.dataset.name + "?", text: "devd forgets the server and the links that deploy to it. The machine and what is on it are not touched.", confirm: "Forget", danger: true }).then(function (ok) { if (ok) opsAct("server.remove", { server: n.dataset.name }); });
    },
    "ops-link-remove": function (n) {
      uiConfirm({ title: "Unlink " + n.dataset.id + "?", text: "Only devd's record goes. Nothing on the server changes.", confirm: "Unlink", danger: true }).then(function (ok) { if (ok) opsAct("remote.unlink", { link: n.dataset.id }); });
    },
    // keys
    "ops-key-public": function (n) {
      read("ssh.public", { name: n.dataset.name }).then(function (r) { uiSheet("Public key " + n.dataset.name, '<pre class="log-block">' + esc(r.public_key) + '</pre><p class="dim">Paste this into the server\'s ~/.ssh/authorized_keys, or into GitHub or your provider.</p>'); }).catch(function (e) { toast("error", e.message); });
    },
    "ops-key-new": function () {
      uiPrompt({ title: "Make an SSH key", text: "A name for the file in ~/.ssh.", confirm: "Make", input: { placeholder: "id_ed25519" } }).then(function (name) {
        if (!name) return;
        run("ssh.keygen", { name: name }).then(function (o) { if (o && o.data) uiSheet("New key " + name, '<pre class="log-block">' + esc(o.data.public_key) + "</pre>"); opsLoad("keys"); });
      });
    },
    "ops-key-adopt": function () {
      uiPrompt({ title: "Take in a private key", text: "The path of the key file you already have, then the name to give it: C:\\keys\\old_server_key old_server. It is copied, not moved.", confirm: "Take in", input: { placeholder: "path name" } }).then(function (v) {
        var w = opsWords(v);
        if (w.length < 2) return toast("error", "write the path, a space, then a name");
        var name = w.pop();
        opsAct("ssh.adopt", { path: w.join(" ").replace(/^"|"$/g, ""), name: name });
      });
    },
    // deploy: remote, variables, schedule, staging
    "ops-link-add": function () {
      uiPrompt({ title: "Deploy " + OPS.site + " to a server", text: "server /folder/on/the/server [releases|git] [domain]. releases ships a build and switches atomically (default); git has the server pull and run your deploy script (for Ploi or Forge machines).", confirm: "Link", input: { placeholder: "hetzner-prod /var/www/shop releases shop.example.com" } }).then(function (v) {
        var w = opsWords(v);
        if (w.length < 2) return toast("error", "write the server and the folder on it");
        opsAct("remote.link", { site: OPS.site, server: w[0], path: w[1], mode: w[2] || null, domain: w[3] || null });
      });
    },
    "ops-remote-deploy": function (n) {
      run("remote.deploy", { link: n.dataset.link, wait: false }).then(function (o) { opsLoad("deploy"); if (o) { setTimeout(function () { opsLoad("deploy"); }, 5000); setTimeout(function () { opsLoad("deploy"); }, 20000); } });
    },
    "ops-remote-releases": function (n) {
      run("remote.releases", { link: n.dataset.link }).then(function (o) {
        if (o && o.data) opsSheetText("Releases of " + n.dataset.link, (o.data.releases || []).map(function (r) { return (r.current ? "-> " : "   ") + r.release + (r.commit ? "  " + r.commit.slice(0, 10) : ""); }).join("\n") || "no releases yet");
      });
    },
    "ops-remote-rollback": function (n) {
      uiConfirm({ title: "Roll " + n.dataset.link + " back?", text: "The previous release goes live with one atomic switch. Database migrations are not undone.", confirm: "Roll back", danger: true }).then(function (ok) { if (ok) opsAct("remote.rollback", { link: n.dataset.link }); });
    },
    "ops-remote-env": function (n) {
      uiConfirm({ title: "Send the .env to the server?", text: "It goes to shared/.env with APP_ENV=production, APP_DEBUG=false and APP_URL set. Review database and mail settings there afterwards. An existing file is not replaced.", confirm: "Send", danger: true }).then(function (ok) { if (ok) run("remote.env_push", { link: n.dataset.link }); });
    },
    "ops-remote-site": function (n) { run("remote.site_create", { link: n.dataset.link }); },
    "ops-remote-ssl": function (n) {
      uiPrompt({ title: "HTTPS certificate", text: "An email address for Let's Encrypt. The domain must already point at the server.", confirm: "Get it", input: { placeholder: "you@example.com" } }).then(function (e) { if (e) run("remote.ssl", { link: n.dataset.link, email: e }); });
    },
    "ops-vars-edit": function () {
      var names = Object.keys((OPS.data.deploy && OPS.data.deploy[5] && OPS.data.deploy[5].vars) || {});
      uiPrompt({ title: "Deploy variables", text: "Write NAME=value to set one, or NAME= to remove it. They are {NAME} in the deploy script and environment variables to its commands." + (names.length ? " Now: " + names.join(", ") : ""), confirm: "Save", input: { placeholder: "NAME=value" } }).then(function (v) {
        if (!v) return;
        var i = v.indexOf("="); if (i < 1) return toast("error", "write NAME=value");
        var vars = {}; vars[v.slice(0, i).trim()] = v.slice(i + 1) === "" ? null : v.slice(i + 1);
        opsAct("deploy.vars_set", { site: OPS.site, vars: vars });
      });
    },
    "ops-sched-add": function () {
      uiPrompt({ title: "Deploy " + OPS.site + " later", text: "in 30m, in 2h, in 1d, or a time like 2026-10-24T09:00:00+07:00. It runs while devd is open; if devd is closed then, it is marked missed.", confirm: "Schedule", input: { placeholder: "in 2h" } }).then(function (v) { if (v) opsAct("deploy.schedule", { site: OPS.site, at: v }); });
    },
    "ops-sched-cancel": function (n) { opsAct("deploy.unschedule", { id: n.dataset.id }); },
    "ops-staging-create": function () {
      uiPrompt({ title: "Make a staging site", text: "A second working folder of this repository on its own branch, served as " + OPS.site + "-staging. Branch name:", confirm: "Make it", input: { value: "staging" } }).then(function (b) { if (b !== null) run("deploy.staging_create", { site: OPS.site, branch: b || null }).then(function () { opsLoad("deploy"); }); });
    },
    "ops-staging-promote": function () {
      uiConfirm({ title: "Ship staging to production?", text: "The staging branch is merged into " + OPS.site + " (fast-forward only) and production is deployed.", confirm: "Ship it", danger: true }).then(function (ok) { if (ok) run("deploy.staging_promote", { site: OPS.site }).then(function () { opsLoad("deploy"); }); });
    },
    "ops-staging-remove": function () {
      uiConfirm({ title: "Remove the staging site?", text: "Its working folder goes, including the copied .env and installed dependencies. The branch stays.", confirm: "Remove", danger: true }).then(function (ok) { if (ok) run("deploy.staging_remove", { site: OPS.site }).then(function () { opsLoad("deploy"); }); });
    }
  });


  boot();
})();
