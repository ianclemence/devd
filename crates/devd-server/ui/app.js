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
    }).then(function (res) {
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

  function errorOf(body, fallback) {
    if (body && body.error && body.error.message) return body.error.message;
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
          return askApproval(res.body).then(function (granted) {
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
              toast("ok", (outcome && outcome.message) || id + " applied");
              return outcome || { ok: true, message: id + " applied" };
            });
          });
        }
        if (!res.ok) {
          toast("error", errorOf(res.body, id + " failed"));
          return null;
        }
        var outcome = res.body.outcome;
        toast("ok", (outcome && outcome.message) || id + " done");
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
    view: "overview",
    session: null,
    doctor: null,
    health: null,
    projects: [],
    project: null,
    tab: "changes",
    tabs: {},
    expanded: {},
    diff: null,
    caps: { total: 0, items: [], q: "", group: "" },
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
    harnesses: [],
    agent: {
      starting: false,
      cwd: null,
      newOpen: false
    },
    sessions: [],
    activeSession: null,
    terms: {},
    agentTimer: null,
    tasks: [],
    delegateOpen: false,
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

  function el(id) {
    return document.getElementById(id);
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

  function statusTag(code) {
    var c = String(code || "").toUpperCase();
    var cls = "tag";
    var letter = c || "?";
    if (c.indexOf("A") >= 0 || c === "?") { cls += " tag-add"; letter = c === "?" ? "U" : "A"; }
    else if (c.indexOf("D") >= 0) { cls += " tag-del"; letter = "D"; }
    else if (c.indexOf("R") >= 0) { cls += " tag-mod"; letter = "R"; }
    else if (c.indexOf("C") >= 0) { cls += " tag-mod"; letter = "C"; }
    else if (c.indexOf("U") >= 0) { cls += " tag-conf"; letter = "U"; }
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

  function toast(kind, message, detail) {
    var node = document.createElement("div");
    node.className = "toast toast-" + kind;
    node.innerHTML =
      '<div class="toast-msg">' + esc(message) +
      (detail ? '<div class="toast-detail">' + esc(detail) + "</div>" : "") +
      '</div><button type="button" data-action="dismiss-toast" aria-label="Dismiss">x</button>';
    el("toasts").appendChild(node);
    setTimeout(function () {
      if (node.parentNode) node.parentNode.removeChild(node);
    }, kind === "error" ? 9000 : 4200);
  }

  // ------------------------------------------------------------------- drawer

  function openDrawer(title, html) {
    el("drawer-title").textContent = title;
    el("drawer-body").innerHTML = html;
    el("drawer").hidden = false;
  }

  function closeDrawer() {
    el("drawer").hidden = true;
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

  function paletteSearch(query) {
    return request("/api/capabilities?limit=60&q=" + encodeURIComponent(query || ""))
      .then(function (res) {
        if (!res.ok) return;
        state.palette.items = res.body.items || [];
        state.palette.index = 0;
        renderPalette();
      });
  }

  function openPalette() {
    state.palette.open = true;
    el("palette").hidden = false;
    var input = el("palette-input");
    input.value = "";
    input.placeholder =
      "Search " + ((state.session && state.session.registry.capabilities) || 0) + " capabilities";
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
      list.innerHTML = '<div class="empty">No capability matches that</div>';
    } else {
      list.innerHTML = items
        .map(function (c, i) {
          var hint = c.requires_approval ? "approval required" : c.tier === "read" ? "Enter runs it" : "Enter inspects it";
          return (
            '<button type="button" class="palette-item' + (i === state.palette.index ? " active" : "") +
            '" data-action="palette-pick" data-index="' + i + '">' +
            '<span><span class="pi-id">' + esc(c.id) + '</span>' +
            '<div class="pi-sum">' + esc(c.summary) + "</div></span>" +
            '<span class="pi-group">' + esc(c.tier) + "</span>" +
            '<span class="pi-group">' + esc(hint) + "</span>" +
            "</button>"
          );
        })
        .join("");
      var active = list.querySelector(".palette-item.active");
      if (active && active.scrollIntoView) active.scrollIntoView({ block: "nearest" });
    }
    el("palette-foot").innerHTML =
      "<span>Up/Down to move</span><span>Enter to act</span><span>Esc to close</span>";
  }

  function paletteActivate(index) {
    var cap = state.palette.items[index];
    if (!cap) return;
    closePalette();
    if (cap.tier === "read" && !cap.requires_approval) {
      read(cap.id, {})
        .then(function (data) {
          openDrawer(cap.id, renderPatch(JSON.stringify(data, null, 2), 2000));
        })
        .catch(function (e) {
          toast("error", e.message);
          openCapability(cap.id);
        });
    } else {
      openCapability(cap.id);
    }
  }

  // -------------------------------------------------------------------- views

  function rail() {
    var server = state.server;
    var running = server.services.filter(function (s) { return s.running; }).length;
    var groups = [
      {
        label: "Workspace",
        items: [
          { view: "projects", label: "Projects", count: state.projects.length || null },
          { view: "agents", label: "Agents", count: state.sessions.filter(function (s) { return s.live_state === "running"; }).length || null },
          { view: "processes", label: "Processes" },
          { view: "shared", label: "Shared" },
          {
            view: "capabilities",
            label: "Capabilities",
            count: (state.session && state.session.registry.capabilities) || null
          },
          { view: "activity", label: "Activity", count: null },
          {
            view: "system",
            label: "System",
            count: state.health && state.health.ok === false ? "!" : null
          }
        ]
      },
      {
        label: "Server",
        items: [
          { view: "overview", label: "Overview" },
          { view: "sites", label: "Sites", count: server.sites.length || null },
          { view: "preview", label: "Preview" },
          {
            view: "services",
            label: "Services",
            count: server.services.length ? running + "/" + server.services.length : null
          },
          { view: "runtimes", label: "Runtimes", count: server.runtimes.length || null },
          { view: "certificates", label: "Certificates" },
          { view: "hosts", label: "Hosts" }
        ]
      }
    ];

    el("rail").innerHTML = groups
      .map(function (group) {
        return (
          '<div class="rail-section">' + esc(group.label) + "</div>" +
          group.items
            .map(function (i) {
              return (
                '<button type="button" class="rail-item' +
                (state.view === i.view || (state.view === "project" && i.view === "projects") ? " active" : "") +
                '" data-action="nav" data-view="' + i.view + '">' +
                "<span>" + esc(i.label) + "</span>" +
                (i.count !== null && i.count !== undefined
                  ? '<span class="rail-count">' + esc(i.count) + "</span>"
                  : "") +
                "</button>"
              );
            })
            .join("")
        );
      })
      .join("");

    var foot = [];
    if (state.session) {
      foot.push('<div class="mono nowrap" title="' + attr(state.session.paths.root) + '">' +
        esc(state.session.paths.root) + "</div>");
    }
    foot.push('<div>version: ' + esc((state.session && state.session.version) || "?") + "</div>");
    el("rail-foot").innerHTML = foot.join("");
  }

  function crumbs() {
    var labels = {
      overview: "Overview",
      sites: "Sites",
      preview: "Preview",
      agents: "Agents",
      processes: "Processes",
      shared: "Shared",
      services: "Services",
      runtimes: "Runtimes",
      certificates: "Certificates",
      hosts: "Hosts",
      projects: "Projects",
      capabilities: "Capabilities",
      activity: "Activity",
      system: "System"
    };
    var parts = [];
    if (state.view === "project" && state.project) {
      parts.push('<button type="button" data-action="nav" data-view="projects">Projects</button>');
      parts.push('<span class="sep">/</span>');
      parts.push("<span>" + esc(state.project.name) + "</span>");
    } else {
      parts.push("<span>" + esc(labels[state.view] || state.view) + "</span>");
    }
    el("crumbs").innerHTML = parts.join("");
  }

  function statusbar() {
    var bits = [];
    if (state.project && state.project.status) {
      var st = state.project.status;
      bits.push("branch " + esc(st.branch || "(detached)"));
      bits.push(st.staged.length + " staged");
      bits.push(st.unstaged.length + " modified");
      bits.push(st.untracked.length + " untracked");
      if (st.conflicted.length) bits.push('<span style="color:var(--danger)">' + st.conflicted.length + " conflicted</span>");
      if (st.ahead) bits.push(st.ahead + " ahead");
      if (st.behind) bits.push(st.behind + " behind");
    }
    if (state.server.loaded) {
      var total = state.server.services.length;
      var up = state.server.services.filter(function (s) { return s.running; }).length;
      if (total) bits.push(up + "/" + total + " services");
      if (state.server.sites.length) {
        bits.push(state.server.sites.length + (state.server.sites.length === 1 ? " site" : " sites"));
      }
    }
    if (state.session) {
      var runningSessions = state.sessions.filter(function (s) { return s.live_state === "running"; }).length;
      if (runningSessions) bits.push(runningSessions + " agent" + (runningSessions === 1 ? "" : "s"));
      bits.push(esc(state.session.registry.capabilities) + " capabilities");
      bits.push("api :" + esc(state.session.port));
    }
    if (state.pending_approvals) bits.push(state.pending_approvals + " pending");
    el("statusbar").innerHTML = bits.join("<span> </span>");
  }

  // -------------------------------------------------------------------- server

  var SERVER_VIEWS = ["overview", "sites", "preview", "services", "runtimes", "certificates", "hosts"];

  var KNOWN_SERVICES = [
    { name: "nginx", label: "nginx", note: "web server" },
    { name: "mariadb", label: "MariaDB", note: "database" },
    { name: "php-cgi", label: "php-cgi", note: "PHP FastCGI listener, built from the installed runtime" }
  ];

  var RUNTIME_KINDS = [
    { kind: "php", label: "PHP", note: "8.x, downloaded from windows.php.net" },
    { kind: "node", label: "Node.js", note: "LTS and current releases" },
    { kind: "go", label: "Go", note: "the official toolchain" }
  ];

  function page(title, sub, actions, body) {
    return (
      '<div class="page">' +
      '<div class="page-head"><div>' +
      '<h1 class="page-title">' + esc(title) + "</h1>" +
      '<div class="page-sub">' + esc(sub) + "</div></div>" +
      '<div class="page-actions">' + (actions || "") + "</div>" +
      "</div>" + body + "</div>"
    );
  }

  function dot(on, warn) {
    return '<span class="dot ' + (on ? "dot-ok" : warn ? "dot-warn" : "dot-off") + '" aria-hidden="true"></span>';
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

  function phpPrimary() {
    var php = state.server.runtimes.filter(function (r) { return r.runtime.kind === "php"; });
    var primary = php.filter(function (r) { return r.primary; })[0];
    if (primary) return primary.runtime.version;
    return php.length ? php[0].runtime.version : null;
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

  function loadServer() {
    return Promise.all([
      read("service.list").catch(function () { return []; }),
      read("site.list").catch(function () { return []; }),
      read("runtime.list").catch(function () { return []; }),
      read("tls.list").catch(function () { return []; }),
      read("tls.ca.show").catch(function () { return null; }),
      read("hosts.show").catch(function () { return null; }),
      read("tls.ca.status").catch(function () { return null; })
    ]).then(function (r) {
      state.server.services = Array.isArray(r[0]) ? r[0] : [];
      state.server.sites = Array.isArray(r[1]) ? r[1] : [];
      state.server.runtimes = Array.isArray(r[2]) ? r[2] : [];
      state.server.certs = Array.isArray(r[3]) ? r[3] : [];
      state.server.ca = r[4];
      state.server.hosts = r[5];
      state.server.trust = r[6];
      state.server.loaded = true;
    });
  }

  // ------------------------------------------------------------------ overview

  function renderOverview() {
    var s = state.server;
    var total = s.services.length;
    var up = s.services.filter(function (x) { return x.running; }).length;
    var healthy = total > 0 && up === total;
    var hosts = s.hosts || {};
    var pending = (hosts.adding || []).length + (hosts.removing || []).length;
    var php = phpPrimary();

    var banner =
      '<div class="banner ' + (healthy ? "banner-ok" : "banner-warn") + '">' +
      dot(healthy, total > 0 && !healthy) +
      '<div class="banner-main">' +
      '<div class="banner-title">' +
      (healthy ? "Server running" : total ? "Server needs attention" : "Nothing installed yet") +
      "</div>" +
      '<div class="banner-sub">' +
      (total ? up + " of " + total + " services running" : "Install PHP, nginx and MariaDB to serve sites") +
      (state.session && state.session.settings
        ? " · HTTP " + state.session.settings.http_port + " · HTTPS " + state.session.settings.https_port
        : "") +
      "</div></div>" +
      '<div class="banner-actions">' +
      '<button type="button" class="btn btn-primary" data-action="stack-up">Start all</button>' +
      '<button type="button" class="btn" data-action="stack-down"' + (up ? "" : " disabled") + ">Stop all</button>" +
      "</div></div>";

    var stats =
      '<div class="stat-grid">' +
      statCard("Services", up + " / " + total, total ? "running" : "none installed") +
      statCard("Sites", String(s.sites.length), s.sites.length ? "served" : "none yet") +
      statCard("PHP", php || "none", php ? "primary runtime" : "not installed") +
      statCard("Certificates", String(s.certs.length), "issued by devd") +
      "</div>";

    var sitesPanel =
      '<div class="panel"><div class="panel-head"><span>Sites</span>' +
      '<span class="panel-note">' + s.sites.length + "</span></div>" +
      (s.sites.length
        ? '<div class="rows">' +
          s.sites
            .map(function (row) {
              var meta =
                '<span class="pill">' + (row.site.tls_mode === "off" ? "http" : "https") + "</span>" +
                (row.site.php ? ' <span class="pill">PHP ' + esc(row.site.php) + "</span>" : "");
              return (
                '<div class="svc">' + dot(row.server_running) +
                '<div class="svc-main"><div class="svc-name">' + esc(row.site.name) + " " + meta + "</div>" +
                '<div class="svc-sub mono">' + esc(row.url) + "</div></div>" +
                '<div class="svc-actions">' +
                '<button type="button" class="btn btn-mini" data-action="open-url" data-url="' + attr(row.url) + '">Open</button>' +
                "</div></div>"
              );
            })
            .join("") + "</div>"
        : '<div class="panel-body empty-inline">No sites yet. ' +
          '<button type="button" class="btn btn-mini" data-action="nav" data-view="sites">Create one</button></div>') +
      "</div>";

    var servicesPanel =
      '<div class="panel"><div class="panel-head"><span>Services</span>' +
      '<span class="panel-note">' + up + " / " + total + "</span></div>" +
      (total
        ? '<div class="rows">' +
          s.services
            .map(function (svc) {
              return (
                '<div class="svc">' + dot(svc.running) +
                '<div class="svc-main"><div class="svc-name">' + esc(svc.name) + "</div>" +
                '<div class="svc-sub mono">' + esc(servicePorts(svc)) + "</div></div>" +
                '<div class="svc-actions">' +
                (svc.running
                  ? '<button type="button" class="btn btn-mini" data-action="restart-service" data-name="' + attr(svc.name) + '">Restart</button>' +
                    '<button type="button" class="btn btn-mini" data-action="stop-service" data-name="' + attr(svc.name) + '">Stop</button>'
                  : '<button type="button" class="btn btn-mini btn-primary" data-action="start-service" data-name="' + attr(svc.name) + '">Start</button>') +
                "</div></div>"
              );
            })
            .join("") + "</div>"
        : '<div class="panel-body empty-inline">Nothing installed. ' +
          '<button type="button" class="btn btn-mini" data-action="nav" data-view="services">Install services</button></div>') +
      "</div>";

    var notices = [];
    if (!total) {
      notices.push(
        notice(
          "No services are installed. devd installs PHP, nginx and MariaDB from their vendors.",
          "Open services",
          "nav",
          "services"
        )
      );
    }
    if (pending) {
      notices.push(
        notice(
          (pending === 1
            ? "1 hostname is not published, so a site on .test does not resolve."
            : pending + " hostnames are not published, so sites on .test do not resolve."),
          "Review hosts",
          "nav",
          "hosts"
        )
      );
    }
    if (s.certs.length && !caTrusted()) {
      notices.push(
        notice(
          "Sites use certificates from devd's local authority. Trust it once so browsers stop warning.",
          "Trust the CA",
          "ca-trust"
        )
      );
    }

    var hostsData = s.hosts || {};
    var published = (hostsData.entries || []).length;
    var pendingNames = (hostsData.adding || []).length + (hostsData.removing || []).length;
    var hostsPanel =
      '<div class="panel"><div class="panel-head"><span>Hosts</span>' +
      '<span class="panel-note">' + published + " published</span></div>" +
      '<div class="panel-body"><div class="inline"><span class="grow dim">' +
      (pendingNames
        ? (pendingNames === 1
            ? "1 change is waiting to be written to the hosts file."
            : pendingNames + " changes are waiting to be written to the hosts file.")
        : "Everything resolves: no site needs a hosts entry.") +
      "</span>" +
      (pendingNames
        ? '<button type="button" class="btn btn-mini" data-action="nav" data-view="hosts">Review</button>'
        : "") +
      "</div></div></div>";

    var certsPanel =
      '<div class="panel"><div class="panel-head"><span>Certificates</span>' +
      '<span class="panel-note">' + s.certs.length + " issued</span></div>" +
      '<div class="panel-body"><div class="inline"><span class="grow dim">' +
      (caTrusted()
        ? "The local authority is trusted by this user."
        : "Trust the local authority once so browsers stop warning.") +
      "</span>" +
      (!caTrusted() && s.certs.length
        ? '<button type="button" class="btn btn-mini" data-action="ca-trust" data-machine="false">Trust</button>'
        : '<button type="button" class="btn btn-mini" data-action="nav" data-view="certificates">Manage</button>') +
      "</div></div></div>";

    el("main").innerHTML = page(
      "Overview",
      "Local services, sites and the state of this machine.",
      '<button type="button" class="btn" data-action="nav" data-view="sites">Sites</button>' +
        '<button type="button" class="btn btn-primary" data-action="new-site">New site</button>',
      banner + stats +
        (notices.length ? '<div class="notice-stack">' + notices.join("") + "</div>" : "") +
        '<div class="overview-grid">' +
        '<div class="overview-stack">' + sitesPanel + hostsPanel + certsPanel + "</div>" +
        servicesPanel +
        "</div>"
    );
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
      "</div>" +
      '<div class="inline" style="margin-top:12px">' +
      '<label class="check"><input type="checkbox" id="site-tls" checked> HTTPS with a local certificate</label>' +
      '<label class="check"><input type="checkbox" id="site-db"> Create a database</label>' +
      '<span class="grow"></span>' +
      '<button type="button" class="btn" data-action="new-site">Cancel</button>' +
      '<button type="button" class="btn btn-primary" data-action="create-site">Create site</button>' +
      "</div></div></div>"
    );
  }

  function renderSites() {
    var s = state.server;
    var rows = s.sites.map(function (row) {
      var site = row.site;
      return (
        "<tr>" +
        '<td><div class="cell-strong">' + esc(site.name) + "</div>" +
        (row.present ? "" : '<div class="cell-warn">directory missing</div>') + "</td>" +
        '<td class="cell-mono"><a href="' + attr(row.url) + '" target="_blank" rel="noreferrer">' + esc(row.url) + "</a></td>" +
        "<td>" + esc(site.php || phpPrimary() || "—") + "</td>" +
        "<td>" +
        (site.tls_mode === "off"
          ? '<span class="pill">http</span>'
          : '<span class="pill ' + (row.certificate ? "pill-ok" : "pill-warn") + '">' +
            (row.certificate ? "https" : "no certificate") + "</span>") +
        "</td>" +
        '<td class="cell-mono nowrap" title="' + attr(row.document_root) + '">' + esc(row.document_root) + "</td>" +
        '<td class="cell-actions">' +
        '<button type="button" class="btn btn-mini" data-action="open-url" data-url="' + attr(row.url) + '">Open</button>' +
        '<button type="button" class="btn btn-mini btn-ghost" data-action="remove-site" data-name="' + attr(site.name) + '">Remove</button>' +
        "</td></tr>"
      );
    });

    var body = state.siteFormOpen ? siteForm() : "";
    if (s.sites.length) {
      body +=
        '<div class="panel"><div class="panel-head"><span>Sites</span>' +
        '<span class="panel-note">' + s.sites.length + "</span></div>" +
        '<div class="panel-body flush"><table class="table"><thead><tr>' +
        "<th>Site</th><th>URL</th><th>PHP</th><th>TLS</th><th>Directory</th><th></th>" +
        "</tr></thead><tbody>" + rows.join("") + "</tbody></table></div></div>";
    } else if (!state.siteFormOpen) {
      body +=
        '<div class="panel"><div class="empty">' +
        '<div class="empty-title">No sites yet</div>' +
        "<div>A site is a project directory, a hostname, a certificate and a URL.</div>" +
        '<div class="inline" style="margin-top:12px">' +
        '<button type="button" class="btn btn-primary" data-action="new-site">Create a site</button>' +
        "</div></div></div>";
    }

    el("main").innerHTML = page(
      "Sites",
      "Projects served over local hostnames.",
      '<button type="button" class="btn btn-primary" data-action="new-site">New site</button>',
      body
    );
  }

  // ------------------------------------------------------------------ services

  function renderServices() {
    var rows = state.server.services;
    var byName = {};
    rows.forEach(function (r) { byName[r.name] = r; });
    var up = rows.filter(function (r) { return r.running; }).length;

    var body = "";
    if (rows.length) {
      body +=
        '<div class="panel"><div class="panel-head"><span>Installed</span>' +
        '<span class="panel-note">' + up + " of " + rows.length + " running</span></div>" +
        '<div class="panel-body flush"><table class="table"><thead><tr>' +
        "<th>Service</th><th>Version</th><th>Ports</th><th>Status</th><th>PID</th><th></th>" +
        "</tr></thead><tbody>" +
        rows
          .map(function (svc) {
            return (
              "<tr>" +
              '<td><div class="cell-strong">' + dot(svc.running) + " " + esc(svc.name) + "</div>" +
              '<div class="cell-dim">' + esc(svc.kind) + "</div></td>" +
              '<td class="cell-mono">' + esc(svc.version || "—") + "</td>" +
              '<td class="cell-mono">' + esc(servicePorts(svc)) + "</td>" +
              "<td>" +
              (svc.running
                ? '<span class="pill pill-ok">running</span>'
                : '<span class="pill">' + (svc.installed ? "stopped" : "files missing") + "</span>") +
              "</td>" +
              '<td class="cell-mono">' + (svc.pid || "—") + "</td>" +
              '<td class="cell-actions">' +
              (svc.running
                ? '<button type="button" class="btn btn-mini" data-action="stop-service" data-name="' + attr(svc.name) + '">Stop</button>' +
                  '<button type="button" class="btn btn-mini" data-action="restart-service" data-name="' + attr(svc.name) + '">Restart</button>'
                : svc.installed
                  ? '<button type="button" class="btn btn-mini btn-primary" data-action="start-service" data-name="' + attr(svc.name) + '">Start</button>'
                  : '<button type="button" class="btn btn-mini btn-primary" data-action="install-service" data-name="' + attr(svc.name) + '">Install</button>') +
              '<button type="button" class="btn btn-mini" data-action="service-logs" data-name="' + attr(svc.name) + '">Logs</button>' +
              '<button type="button" class="btn btn-mini btn-ghost" data-action="remove-service" data-name="' + attr(svc.name) + '">Remove</button>' +
              "</td></tr>"
            );
          })
          .join("") +
        "</tbody></table></div></div>";
    }

    var missing = KNOWN_SERVICES.filter(function (k) { return !byName[k.name]; });
    if (missing.length) {
      body +=
        '<div class="panel"><div class="panel-head"><span>Available</span>' +
        '<span class="panel-note">downloaded from the vendor when installed</span></div>' +
        '<div class="panel-body"><div class="grid grid-3">' +
        missing
          .map(function (k) {
            return (
              '<div class="card">' +
              '<div class="card-title">' + esc(k.label) + "</div>" +
              '<div class="card-meta">' + esc(k.note) + "</div>" +
              '<div class="card-actions">' +
              (k.name === "php-cgi"
                ? '<button type="button" class="btn btn-mini" data-action="nav" data-view="runtimes">Install a PHP runtime</button>'
                : '<button type="button" class="btn btn-mini btn-primary" data-action="install-service" data-name="' + attr(k.name) + '">Install</button>') +
              "</div></div>"
            );
          })
          .join("") +
        "</div></div></div>";
    }

    if (!body) {
      body =
        '<div class="panel"><div class="empty">' +
        '<div class="empty-title">No services</div>' +
        "<div>Install nginx, MariaDB and a PHP runtime to serve sites.</div></div></div>";
    }

    el("main").innerHTML = page(
      "Services",
      "The processes that serve your sites, with their ports, logs and state.",
      '<button type="button" class="btn" data-action="stack-down">Stop all</button>' +
        '<button type="button" class="btn btn-primary" data-action="stack-up">Start all</button>',
      body
    );
  }

  // ------------------------------------------------------------------ runtimes

  function renderRuntimes() {
    var cards = RUNTIME_KINDS.map(function (kind) {
      var installed = state.server.runtimes.filter(function (r) { return r.runtime.kind === kind.kind; });
      var list = installed.length
        ? '<div class="rows">' +
          installed
            .map(function (r) {
              var rt = r.runtime;
              return (
                '<div class="svc">' +
                '<div class="svc-main"><div class="svc-name">' + esc(rt.version) +
                (r.primary ? ' <span class="pill pill-accent">primary</span>' : "") + "</div>" +
                '<div class="svc-sub mono nowrap" title="' + attr(rt.path) + '">' + esc(rt.path) + "</div></div>" +
                '<div class="svc-actions">' +
                (kind.kind === "php" && !r.primary
                  ? '<button type="button" class="btn btn-mini" data-action="use-runtime" data-version="' + attr(rt.version) + '">Use for new sites</button>'
                  : "") +
                '<button type="button" class="btn btn-mini btn-danger" data-action="remove-runtime" data-kind="' + attr(kind.kind) + '" data-version="' + attr(rt.version) + '">Remove</button>' +
                "</div></div>"
              );
            })
            .join("") + "</div>"
        : '<div class="panel-body empty-inline">Not installed.</div>';

      return (
        '<div class="panel"><div class="panel-head"><span>' + esc(kind.label) + "</span>" +
        '<span class="panel-note">' + installed.length + " installed</span></div>" +
        list +
        '<div class="panel-body panel-foot"><div class="inline">' +
        '<span class="dim grow">' + esc(kind.note) + "</span>" +
        '<button type="button" class="btn btn-primary" data-action="install-runtime" data-kind="' + attr(kind.kind) + '">Install latest</button>' +
        "</div></div></div>"
      );
    });

    el("main").innerHTML = page(
      "Runtimes",
      "Interpreters installed on this machine, downloaded from their vendors and verified against a published digest.",
      "",
      cards.join("")
    );
  }

  // -------------------------------------------------------------- certificates

  function renderCertificates() {
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

    var caPanel =
      '<div class="panel"><div class="panel-head"><span>Local certificate authority</span>' +
      '<span class="panel-note">' + caTrustLabel() + "</span></div>" +
      '<div class="panel-body">' +
      (ca
        ? '<div class="kv"><dt>Certificate</dt><dd>' + esc(ca.path) + "</dd></div>" +
          '<div class="inline" style="margin-top:12px">' +
          '<button type="button" class="btn btn-primary" data-action="ca-trust" data-machine="false">Trust for this user</button>' +
          '<button type="button" class="btn" data-action="ca-trust" data-machine="true">Trust for all users</button>' +
          '<button type="button" class="btn btn-ghost" data-action="ca-untrust">Remove trust</button>' +
          "</div>" +
          '<div class="row-sub" style="margin-top:8px">Trusting adds the certificate to the Windows certificate store. ' +
          "The machine-wide option needs an administrator; the per-user option does not.</div>"
        : '<div class="notice notice-warn">No certificate authority yet. Creating an HTTPS site makes one.</div>') +
      "</div></div>";

    var rows = s.certs
      .map(function (c) {
        return (
          "<tr><td class=\"cell-strong\">" + esc(c.host) + "</td>" +
          '<td class="cell-mono nowrap">' + esc(c.cert) + "</td>" +
          "<td>" + (c.key_present
            ? '<span class="pill pill-ok">key present</span>'
            : '<span class="pill pill-bad">key missing</span>') + "</td></tr>"
        );
      })
      .join("");

    var issued =
      '<div class="panel"><div class="panel-head"><span>Issued certificates</span>' +
      '<span class="panel-note">' + s.certs.length + "</span></div>" +
      (rows
        ? '<div class="panel-body flush"><table class="table"><thead><tr>' +
          "<th>Host</th><th>Chain</th><th>Key</th></tr></thead><tbody>" + rows + "</tbody></table></div>"
        : '<div class="panel-body empty-inline">No certificates yet. An HTTPS site issues one.</div>') +
      "</div>";

    el("main").innerHTML = page(
      "Certificates",
      "devd runs its own authority. Every HTTPS site gets a certificate signed by it.",
      '<button type="button" class="btn" data-action="cert-form">Issue for a hostname</button>',
      form + caPanel + issued
    );
  }

  // --------------------------------------------------------------------- hosts

  function renderHosts() {
    var data = state.server.hosts || { adding: [], removing: [], entries: [], file: "", changes: false };
    var adding = data.adding || [];
    var removing = data.removing || [];
    var entries = data.entries || [];
    var pending = adding.length + removing.length;

    var head =
      '<div class="panel"><div class="panel-head"><span>Hosts file</span>' +
      '<span class="panel-note">' + pending + " change(s) pending</span></div>" +
      '<div class="panel-body"><div class="kv">' +
      "<dt>Path</dt><dd>" + esc(data.file) + "</dd>" +
      "<dt>Published</dt><dd>" + entries.length + " name(s)</dd>" +
      "</div>" +
      '<div class="row-sub" style="margin-top:8px">Writing the hosts file needs an elevated shell on Windows. ' +
      "devd snapshots it first and preserves every line it did not write.</div></div></div>";

    var changes = "";
    if (pending) {
      changes =
        '<div class="panel"><div class="panel-head"><span>Pending changes</span>' +
        '<span class="panel-note">' + pending + "</span></div>" +
        '<div class="panel-body flush"><table class="table"><thead><tr>' +
        "<th></th><th>Address</th><th>Host</th></tr></thead><tbody>" +
        adding
          .map(function (e) {
            return '<tr><td><span class="tag tag-add">add</span></td>' +
              '<td class="cell-mono">' + esc(e.address) + "</td>" +
              '<td class="cell-mono">' + esc(e.host) + "</td></tr>";
          })
          .join("") +
        removing
          .map(function (e) {
            return '<tr><td><span class="tag tag-del">remove</span></td>' +
              '<td class="cell-mono">' + esc(e.address) + "</td>" +
              '<td class="cell-mono">' + esc(e.host) + "</td></tr>";
          })
          .join("") +
        "</tbody></table></div></div>";
    }

    var published = entries.length
      ? '<div class="panel"><div class="panel-head"><span>Published names</span>' +
        '<span class="panel-note">' + entries.length + "</span></div>" +
        '<div class="panel-body flush"><table class="table"><thead><tr>' +
        "<th>Address</th><th>Host</th></tr></thead><tbody>" +
        entries
          .map(function (e) {
            return '<tr><td class="cell-mono">' + esc(e.address) + "</td>" +
              '<td class="cell-mono">' + esc(e.host) + "</td></tr>";
          })
          .join("") +
        "</tbody></table></div></div>"
      : '<div class="panel"><div class="panel-body empty-inline">No names published. ' +
        "Sites on localhost resolve without one.</div></div>";

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

  function loadAgents() {
    return Promise.all([
      request("/api/terminal/harnesses").catch(function () { return { ok: false, body: [] }; }),
      request(projectQuery("/api/sessions")).catch(function () { return { ok: false, body: null }; }),
      request(projectQuery("/api/tasks")).catch(function () { return { ok: false, body: null }; })
    ]).then(function (r) {
      state.harnesses = r[0].ok && Array.isArray(r[0].body) ? r[0].body : [];
      state.sessions = r[1].ok && r[1].body && Array.isArray(r[1].body.items) ? r[1].body.items : [];
      state.tasks = r[2].ok && r[2].body && Array.isArray(r[2].body.items) ? r[2].body.items : [];

      var ids = state.sessions.map(function (s) { return s.id; });
      var saved = null;
      try { saved = localStorage.getItem("devd.activeSession"); } catch (e) {}
      if (!state.activeSession || ids.indexOf(state.activeSession) === -1) {
        var pick = saved && ids.indexOf(saved) !== -1 ? saved : null;
        if (!pick) {
          var attachable = state.sessions.filter(function (s) { return s.attachable; })[0];
          pick = attachable ? attachable.id : (state.sessions[0] ? state.sessions[0].id : null);
        }
        state.activeSession = pick;
      }
      if (state.activeSession) {
        try { localStorage.setItem("devd.activeSession", state.activeSession); } catch (e) {}
      }
    });
  }

  function activeSession() {
    if (!state.activeSession) return null;
    return state.sessions.filter(function (s) { return s.id === state.activeSession; })[0] || null;
  }

  function cwdChoices() {
    var out = [];
    state.projects.forEach(function (p) { out.push({ path: p.path, label: p.name }); });
    state.server.sites.forEach(function (row) {
      out.push({ path: row.document_root, label: row.site.name + " (site)" });
    });
    if (state.session) out.push({ path: state.session.paths.www, label: "www" });
    var seen = {};
    return out.filter(function (c) {
      if (!c.path || seen[c.path]) return false;
      seen[c.path] = true;
      return true;
    });
  }

  function agentLabel(kind) {
    var found = state.harnesses.filter(function (h) { return h.kind === kind; })[0];
    return found ? found.label : kind;
  }

  function sessionPill(s) {
    var live = s.live_state || s.state || "unknown";
    var cls = live === "running" ? "pill-ok"
      : live === "failed" ? "pill-bad"
      : live === "completed" ? "pill-accent"
      : (live === "stale" || live === "unavailable") ? "pill-warn" : "";
    return '<span class="pill ' + cls + '">' + esc(live) + "</span>";
  }

  function shortTime(value) {
    if (!value) return "";
    var d = new Date(value);
    if (isNaN(d.getTime())) return "";
    return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }

  function sessionRow(s) {
    var active = s.id === state.activeSession;
    var detail = s.cwd || "";
    var sub = [agentLabel(s.harness), shortTime(s.last_activity_at)].filter(Boolean).join(" · ");
    return (
      '<button type="button" class="session-row' + (active ? " active" : "") +
      '" data-action="open-session" data-id="' + attr(s.id) + '" title="' + attr(detail) + '">' +
      '<span class="session-row-top"><span class="session-dot ' +
      (s.live_state === "running" ? "on" : s.live_state === "failed" ? "bad" : "") +
      '"></span><span class="session-name">' + esc(s.label || s.harness) + "</span>" +
      (s.task_id ? '<span class="pill pill-accent">task</span>' : "") +
      "</span>" +
      '<span class="session-row-sub"><span>' + esc(sub) + "</span>" + sessionPill(s) + "</span>" +
      '<span class="session-row-path">' + esc(detail) + "</span>" +
      "</button>"
    );
  }

  function renderAgents() {
    var a = state.agent;
    if (a.starting) {
      el("main").innerHTML = page("Agents", "Starting the harness…", "", '<div class="loading">Starting the harness…</div>');
      return;
    }

    var choices = cwdChoices();
    var selectedCwd = a.cwd || (state.project && state.project.path) ||
      (choices[0] && choices[0].path) || "";
    var options = choices.map(function (c) {
      return '<option value="' + attr(c.path) + '"' + (selectedCwd === c.path ? " selected" : "") + ">" +
        esc(c.label) + " — " + esc(c.path) + "</option>";
    }).join("");

    var rows = state.sessions.length
      ? state.sessions.map(sessionRow).join("")
      : '<div class="empty-inline">No sessions yet. Start a harness to create one.</div>';

    var tabs = state.sessions.slice(0, 10).map(function (s) {
      return '<button type="button" class="term-tab' + (s.id === state.activeSession ? " active" : "") +
        '" data-action="open-session" data-id="' + attr(s.id) + '" title="' + attr(s.cwd) + '">' +
        '<span class="session-dot ' + (s.live_state === "running" ? "on" : s.live_state === "failed" ? "bad" : "") + '"></span>' +
        esc(s.label || s.harness) +
        (s.live_state === "running" ? "" : ' <span class="tab-state">' + esc(s.live_state) + "</span>") +
        "</button>";
    }).join("");

    var active = activeSession();
    var terminal;
    if (active) {
      var live = active.live_state || active.state;
      var note = active.attachable
        ? "Live PTY owned by this devd process. Switching views does not stop it."
        : (active.unattached_reason || "This session is finished. Its transcript remains readable.");
      terminal =
        '<div class="panel terminal-panel">' +
        '<div class="panel-head terminal-head">' +
        '<div class="terminal-title"><span class="session-name">' + esc(active.label) + "</span>" +
        sessionPill(active) + '<span class="panel-note mono">' + esc(active.cwd) + "</span></div>" +
        '<div class="inline">' +
        '<button type="button" class="btn btn-mini" data-action="rename-session" data-id="' + attr(active.id) + '">Rename</button>' +
        (active.attachable
          ? '<button type="button" class="btn btn-mini" data-action="stop-session" data-id="' + attr(active.id) + '">Stop</button>'
          : '<button type="button" class="btn btn-mini" data-action="dismiss-session" data-id="' + attr(active.id) + '">Dismiss</button>') +
        "</div></div>" +
        '<div class="terminal-note">' + esc(note) + "</div>" +
        '<div class="terminal-shell" id="terminal-slot" data-session="' + attr(active.id) + '"></div>' +
        "</div>";
    } else {
      var cards = state.harnesses.map(function (h) {
        return '<button type="button" class="card agent-card" data-action="new-session" data-kind="' + attr(h.kind) + '"' +
          (h.available ? "" : " disabled") + ">" +
          '<div class="card-title">' + esc(h.label) + "</div>" +
          '<div class="card-meta">' + (h.available ? "installed" : "not installed") + "</div>" +
          '<div class="agent-detail">' + esc(h.detail) + "</div>" +
          '<div class="card-actions"><span class="btn btn-primary">Start</span></div></button>';
      }).join("");
      terminal =
        '<div class="panel"><div class="panel-head"><span>Start a session</span></div>' +
        '<div class="panel-body"><label class="field-label" for="agent-new-cwd">Working directory</label>' +
        '<select class="select mono" id="agent-new-cwd">' + options + "</select>" +
        '<div class="row-sub" style="margin-top:8px">Each session is its own real process and PTY. Switching tabs, sections or reloading the window does not stop it.</div>' +
        '<div class="grid grid-3" style="margin-top:14px">' + cards + "</div></div></div>";
    }

    var taskRows = state.tasks.length
      ? state.tasks.map(function (t) {
          var running = t.state === "running" || t.state === "starting" || t.state === "queued";
          return '<div class="task-row"><div class="task-main"><div class="task-title">' +
            esc(t.title) + " " + '<span class="pill ' + (t.state === "succeeded" ? "pill-ok" : t.state === "failed" ? "pill-bad" : "") + '">' + esc(t.state) + "</span>" +
            '</div><div class="task-sub mono">' + esc(t.harness) + " · depth " + esc(t.depth) + " · " + esc(t.cwd) + "</div>" +
            (t.error ? '<div class="task-error">' + esc(t.error) + "</div>" : "") + "</div>" +
            '<div class="inline">' +
            (running ? '<button type="button" class="btn btn-mini" data-action="task-wait" data-id="' + attr(t.id) + '">Wait</button>' +
              '<button type="button" class="btn btn-mini" data-action="task-cancel" data-id="' + attr(t.id) + '">Cancel</button>' : "") +
            '<button type="button" class="btn btn-mini" data-action="task-result" data-id="' + attr(t.id) + '">Result</button>' +
            "</div></div>";
        }).join("")
      : '<div class="empty-inline">No delegated tasks. A coordinating agent can spawn one with agent.spawn, or use Delegate here.</div>';

    el("main").innerHTML = page(
      "Agents",
      "Real PTYs owned by the backend. Closing a view or reloading the window does not restart the harness.",
      '<button type="button" class="btn" data-action="delegate-open">Delegate a task</button>' +
        '<button type="button" class="btn btn-primary" data-action="new-session" data-kind="shell">New session</button>',
      '<div class="agents-layout">' +
        '<aside class="panel session-side">' +
        '<div class="panel-head"><span>Sessions</span><span class="panel-note">' + state.sessions.length + "</span></div>" +
        '<div class="session-list">' + rows + "</div>" +
        '<div class="session-side-foot"><select class="select mono" id="agent-side-cwd">' + options + "</select>" +
        '<div class="inline" style="margin-top:8px">' +
        state.harnesses.map(function (h) {
          return '<button type="button" class="btn btn-mini" data-action="new-session" data-kind="' + attr(h.kind) + '"' +
            (h.available ? "" : " disabled title=\"not installed\"") + ">" + esc(h.label) + "</button>";
        }).join("") +
        "</div></div></aside>" +
        '<section class="session-main">' +
        '<div class="term-tabs">' + (tabs || '<span class="faint">No tabs</span>') + "</div>" +
        terminal +
        '<div class="panel"><div class="panel-head"><span>Delegated tasks</span><span class="panel-note">' +
        state.tasks.filter(function (t) { return t.state === "running" || t.state === "starting"; }).length + " running</span></div>" +
        '<div class="panel-body flush">' + taskRows + "</div></div>" +
        "</section></div>"
    );

    attachActive();
  }

  function ensureTerm(id) {
    if (state.terms[id]) return state.terms[id];
    var term = new window.Terminal({
      fontFamily: 'ui-monospace, "Cascadia Mono", Consolas, monospace',
      fontSize: 12,
      lineHeight: 1.35,
      cursorBlink: true,
      scrollback: 5000,
      theme: {
        background: "#0a0a0b",
        foreground: "#e7e5e4",
        cursor: "#e07856",
        selectionBackground: "#3a2a22",
        black: "#1c1917",
        red: "#f0a58c",
        green: "#9ac9a0",
        yellow: "#e6c07b",
        blue: "#8ab4d8",
        magenta: "#c9a3d8",
        cyan: "#8fc7c7",
        white: "#e7e5e4"
      }
    });
    var fit = new window.FitAddon.FitAddon();
    term.loadAddon(fit);
    term.onData(function (data) {
      post("/api/sessions/" + encodeURIComponent(id) + "/input", { data: data }).then(function (res) {
        if (!res.ok) markSessionGone(id, res.body);
      });
    });
    term.onResize(function (size) {
      post("/api/sessions/" + encodeURIComponent(id) + "/resize", { cols: size.cols, rows: size.rows });
    });
    state.terms[id] = { term: term, fit: fit, cursor: 0, opened: false, attachable: true, finished: false };
    return state.terms[id];
  }

  function markSessionGone(id, body) {
    var t = state.terms[id];
    if (t) t.attachable = false;
    var s = state.sessions.filter(function (x) { return x.id === id; })[0];
    if (s) s.attachable = false;
    if (body) {
      var msg = body.error && body.error.message;
      if (msg) toast("warn", msg);
    }
  }

  function attachActive() {
    var active = activeSession();
    var slot = el("terminal-slot");
    if (!active || !slot) {
      stopAgentTimer();
      return;
    }
    var t = ensureTerm(active.id);
    t.attachable = !!active.attachable;
    if (!t.opened) {
      t.term.open(slot);
      t.opened = true;
    } else if (t.term.element && t.term.element.parentNode !== slot) {
      slot.appendChild(t.term.element);
    }
    try { t.fit.fit(); } catch (e) {}
    t.term.focus();
    if (!t.finished) fetchOutput(active.id);
    if (active.attachable) startAgentTimer(active.id);
    else stopAgentTimer();
  }

  function fetchOutput(id) {
    var t = state.terms[id];
    if (!t) return;
    post("/api/sessions/" + encodeURIComponent(id) + "/output", { cursor: t.cursor }).then(function (res) {
      if (!res.ok || !res.body) return;
      var body = res.body;
      if (body.reset) {
        try { t.term.reset(); } catch (e) {}
      }
      if (body.data) {
        var raw = atob(body.data);
        var bytes = new Uint8Array(raw.length);
        for (var i = 0; i < raw.length; i++) bytes[i] = raw.charCodeAt(i);
        t.term.write(bytes);
      }
      t.cursor = body.cursor || t.cursor;
      if (body.exited) {
        t.attachable = false;
        if (!t.finished) {
          t.finished = true;
          stopAgentTimer();
          var s = state.sessions.filter(function (x) { return x.id === id; })[0];
          if (s) {
            s.attachable = false;
            s.live_state = body.state === "finished"
              ? (body.code === 0 ? "completed" : "failed")
              : (body.state || "completed");
            s.exit_code = body.code;
          }
          toast(body.code === 0 ? "ok" : "warn", "Session ended" + (body.code === 0 ? "" : " with code " + body.code));
          loadAgents().then(renderAgents);
        }
      }
    }).catch(function () {});
  }

  function startAgentTimer(id) {
    stopAgentTimer();
    state.agentTimer = setInterval(function () {
      if (state.view !== "agents" || state.activeSession !== id) {
        stopAgentTimer();
        return;
      }
      var t = state.terms[id];
      if (!t || !t.attachable) {
        stopAgentTimer();
        return;
      }
      fetchOutput(id);
    }, 110);
  }

  function stopAgentTimer() {
    if (state.agentTimer) {
      clearInterval(state.agentTimer);
      state.agentTimer = null;
    }
  }

  function openSession(id) {
    state.activeSession = id;
    try { localStorage.setItem("devd.activeSession", id); } catch (e) {}
    render();
  }

  function newSession(kind) {
    var cwd = readInput("agent-new-cwd") || readInput("agent-side-cwd") || state.agent.cwd;
    if (!cwd) {
      var choices = cwdChoices();
      cwd = choices[0] && choices[0].path;
    }
    if (!cwd) return toast("warn", "Add a project or choose a working directory first");
    state.agent.cwd = cwd;
    state.agent.starting = true;
    closeDrawer();
    render();
    post("/api/sessions", {
      kind: kind,
      cwd: cwd,
      project: state.project ? state.project.path : null,
      cols: 100,
      rows: 30
    }).then(function (res) {
      state.agent.starting = false;
      if (!res.ok || !res.body) {
        toast("error", errorOf(res.body, "could not start " + kind));
        render();
        return;
      }
      var id = res.body.id || (res.body.session && res.body.session.id);
      state.activeSession = id;
      try { localStorage.setItem("devd.activeSession", id); } catch (e) {}
      loadAgents().then(function () {
        state.activeSession = id;
        render();
      });
    });
  }

  function openNewSession() {
    var choices = cwdChoices();
    var options = choices.map(function (c) {
      return '<option value="' + attr(c.path) + '">' + esc(c.label) + " — " + esc(c.path) + "</option>";
    }).join("");
    var cards = state.harnesses.map(function (h) {
      return '<button type="button" class="card agent-card" data-action="new-session" data-kind="' + attr(h.kind) + '"' +
        (h.available ? "" : " disabled") + '><div class="card-title">' + esc(h.label) + "</div>" +
        '<div class="card-meta">' + (h.available ? "installed" : "not installed") + "</div>" +
        '<div class="agent-detail">' + esc(h.detail) + "</div></button>";
    }).join("");
    openDrawer("New agent session",
      '<div class="stack"><div class="field"><label class="field-label" for="agent-new-cwd">Working directory</label>' +
      '<select class="select mono" id="agent-new-cwd">' + options + "</select></div>" +
      '<div class="grid grid-3">' + (cards || '<div class="faint">No harnesses found.</div>') + "</div>" +
      '<div class="faint">The harness asks its own permission questions. devd supplies the directory, the PTY and the process.</div></div>');
  }

  function stopSession(id) {
    if (!window.confirm("Stop this session and its child processes? Its transcript will be kept.")) return;
    post("/api/sessions/" + encodeURIComponent(id) + "/stop", {}).then(function (res) {
      if (!res.ok) return toast("error", errorOf(res.body, "could not stop the session"));
      var t = state.terms[id];
      if (t) {
        try { t.term.dispose(); } catch (e) {}
        delete state.terms[id];
      }
      toast("ok", "Session stopped; its output is still readable");
      loadAgents().then(renderAgents);
    });
  }

  function dismissSession(id) {
    if (!window.confirm("Dismiss this finished session from the list? The transcript file is retained.")) return;
    request("/api/sessions/" + encodeURIComponent(id), { method: "DELETE", body: "{}" }).then(function (res) {
      if (!res.ok) return toast("error", errorOf(res.body, "could not dismiss the session"));
      var t = state.terms[id];
      if (t) {
        try { t.term.dispose(); } catch (e) {}
        delete state.terms[id];
      }
      if (state.activeSession === id) state.activeSession = null;
      loadAgents().then(renderAgents);
    });
  }

  function renameSession(id) {
    var s = state.sessions.filter(function (x) { return x.id === id; })[0];
    openDrawer("Rename session",
      '<div class="stack"><div class="field"><label class="field-label" for="session-label">Label</label>' +
      '<input class="input" id="session-label" value="' + attr(s ? s.label : "") + '"></div>' +
      '<button type="button" class="btn btn-primary" data-action="save-session-label" data-id="' + attr(id) + '">Save</button></div>');
  }

  function saveSessionLabel(id) {
    var label = readInput("session-label");
    if (!label) return toast("warn", "A session needs a label");
    post("/api/sessions/" + encodeURIComponent(id) + "/rename", { label: label }).then(function (res) {
      if (!res.ok) return toast("error", errorOf(res.body, "could not rename the session"));
      closeDrawer();
      loadAgents().then(renderAgents);
    });
  }

  function openDelegate() {
    var choices = cwdChoices();
    var options = choices.map(function (c) {
      return '<option value="' + attr(c.path) + '">' + esc(c.path) + "</option>";
    }).join("");
    var harnessOptions = state.harnesses.filter(function (h) { return h.kind === "claude" || h.kind === "opencode"; })
      .map(function (h) {
        return '<option value="' + attr(h.kind) + '"' + (h.available ? "" : " disabled") + ">" +
          esc(h.label) + (h.available ? "" : " (not installed)") + "</option>";
      }).join("");
    openDrawer("Delegate a task",
      '<div class="stack">' +
      '<div class="field"><label class="field-label" for="task-title">Title</label><input class="input" id="task-title" placeholder="Investigate the failing test"></div>' +
      '<div class="field"><label class="field-label" for="task-harness">Agent</label><select class="select" id="task-harness">' + harnessOptions +
      '<option value="custom">Custom command (argv JSON)</option></select></div>' +
      '<div class="field"><label class="field-label" for="task-cwd">Working directory</label><select class="select mono" id="task-cwd">' + options + "</select></div>" +
      '<div class="field"><label class="field-label" for="task-instructions">Instructions</label><textarea class="input mono" id="task-instructions" rows="6" placeholder="What should the child do?"></textarea></div>' +
      '<div class="field"><label class="field-label" for="task-command">Custom command (only for custom)</label><input class="input mono" id="task-command" placeholder=\'["node","script.js"]\'></div>' +
      '<div class="field"><label class="field-label" for="task-timeout">Timeout (seconds)</label><input class="input" id="task-timeout" value="300"></div>' +
      '<button type="button" class="btn btn-primary" data-action="delegate-go">Spawn a real child process</button>' +
      '<div class="faint">The child is a separate PTY, limited to three levels of nesting and eight unfinished tasks. A wait timeout does not kill it.</div></div>');
  }

  function delegateTask() {
    var title = readInput("task-title");
    var harness = readInput("task-harness");
    var cwd = readInput("task-cwd");
    var instructions = el("task-instructions") ? el("task-instructions").value : "";
    var commandText = readInput("task-command");
    var timeout = Number(readInput("task-timeout")) || 300;
    if (!title) return toast("warn", "A task needs a title");
    if (!cwd) return toast("warn", "A task needs a working directory");
    var command = null;
    if (harness === "custom") {
      try {
        command = JSON.parse(commandText);
      } catch (e) {
        return toast("error", "The custom command must be a JSON array of arguments");
      }
      if (!Array.isArray(command) || !command.length) {
        return toast("error", "The custom command must be a non-empty JSON array");
      }
    }
    var active = activeSession();
    post("/api/invoke", {
      id: "agent.spawn",
      args: {
        title: title,
        harness: harness,
        cwd: cwd,
        instructions: instructions,
        project: state.project ? state.project.path : cwd,
        parent_session_id: active ? active.id : null,
        timeout_secs: timeout,
        command: command
      },
      dry_run: false
    }).then(function (res) {
      if (!res.ok) return toast("error", errorOf(res.body, "could not delegate the task"));
      var data = res.body.outcome && res.body.outcome.data;
      var task = data && data.task;
      closeDrawer();
      toast("ok", "Delegated " + (task ? task.title : title));
      loadAgents().then(renderAgents);
    });
  }

  function waitTask(id) {
    toast("ok", "Waiting for the delegated task…");
    post("/api/tasks/" + encodeURIComponent(id) + "/wait", { timeout_secs: 120 }).then(function (res) {
      if (!res.ok) return toast("error", errorOf(res.body, "the wait failed"));
      if (res.body.timed_out) toast("warn", "The task is still running; it was not killed");
      else toast("ok", "Task " + res.body.task.state);
      loadAgents().then(renderAgents);
    });
  }

  function cancelTask(id) {
    if (!window.confirm("Cancel this delegated task and stop its child process?")) return;
    post("/api/tasks/" + encodeURIComponent(id) + "/cancel", {}).then(function (res) {
      if (!res.ok) return toast("error", errorOf(res.body, "could not cancel the task"));
      toast("ok", "Task cancelled");
      loadAgents().then(renderAgents);
    });
  }

  function resultTask(id) {
    request("/api/tasks/" + encodeURIComponent(id) + "/result").then(function (res) {
      if (!res.ok) return toast("error", errorOf(res.body, "could not read the result"));
      var task = res.body.task || {};
      var output = String(res.body.output || "").replace(/\u001b\][^\u0007]*\u0007/g, "").replace(/\u001b\[[0-9;?]*[A-Za-z]/g, "");
      openDrawer("Task result — " + (task.title || id),
        '<div class="stack"><div class="inline">' + sessionPill({ live_state: task.state }) +
        '<span class="pill">exit ' + esc(task.exit_code === null || task.exit_code === undefined ? "?" : task.exit_code) + "</span>" +
        (task.error ? '<span class="pill pill-bad">' + esc(task.error) + "</span>" : "") + "</div>" +
        '<pre class="log-pre">' + esc(output || "(no output captured)") + "</pre></div>");
    });
  }

  // ----------------------------------------------------------------- processes

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
    var p = state.processes;
    var project = processProject();
    p.project = project;
    if (project && !p.plan && !p.error && !p.loading) {
      p.loading = true;
      loadProcesses().then(renderProcesses);
    }
    if (p.loading && !p.plan) {
      el("main").innerHTML = page("Processes", "Supervised project processes from devd.yml.", "", '<div class="loading">Reading the manifest…</div>');
      return;
    }

    var select = '<div class="panel"><div class="panel-head"><span>Project manifest</span></div><div class="panel-body">' +
      '<select class="select mono" id="process-project">' + (projectOptions(project) || '<option value="">No projects</option>') + "</select>" +
      '<div class="row-sub" style="margin-top:8px">devd reads <span class="mono">devd.yml</span> from the project root. Nothing runs until you review and approve the exact commands.</div></div></div>';

    if (!project || (p.error && !p.plan)) {
      el("main").innerHTML = page("Processes", "Supervised project processes from devd.yml.", "", select +
        '<div class="panel"><div class="empty"><div class="empty-title">No manifest to run</div>' +
        "<div>" + esc(p.error || "Choose a project that contains devd.yml.") + "</div>" +
        '<pre class="log-pre" style="text-align:left;margin-top:14px">version: 1\nproject:\n  name: example-app\nprocesses:\n  web:\n    command: ["npm", "run", "dev"]\n    working_dir: "."\n    health:\n      url: "http://127.0.0.1:3000/"</pre></div></div>');
      return;
    }

    var plan = p.plan || { processes: [] };
    var status = (p.status && p.status.processes) || [];
    var byName = {};
    status.forEach(function (row) { byName[row.name] = row; });

    var approval = plan.approved
      ? '<span class="pill pill-ok">approved</span><span class="panel-note mono">' + esc(String(plan.hash || "").slice(0, 12)) + "</span>"
      : '<span class="pill pill-warn">approval required</span>' +
        '<button type="button" class="btn btn-primary btn-mini" data-action="approve-project">Approve these commands</button>';

    var rows = (plan.processes || []).map(function (spec) {
      var row = byName[spec.name] || {};
      var stateName = row.status || "stopped";
      var health = row.health || (spec.health_declared ? "unknown" : "none");
      var actions = "";
      if (stateName === "running" || stateName === "starting") {
        actions += '<button type="button" class="btn btn-mini" data-action="process-stop" data-name="' + attr(spec.name) + '">Stop</button>' +
          '<button type="button" class="btn btn-mini" data-action="process-restart" data-name="' + attr(spec.name) + '">Restart</button>';
      } else {
        actions += '<button type="button" class="btn btn-mini btn-primary" data-action="process-start" data-name="' + attr(spec.name) + '"' +
          (plan.approved ? "" : " disabled") + ">Start</button>";
      }
      actions += '<button type="button" class="btn btn-mini" data-action="process-logs" data-name="' + attr(spec.name) + '">Logs</button>';
      return '<tr><td><div class="cell-strong">' + esc(spec.name) + "</div>" +
        '<div class="cell-mono">' + esc((spec.command || []).join(" ")) + "</div></td>" +
        '<td><span class="pill ' + (stateName === "running" ? "pill-ok" : stateName === "failed" ? "pill-bad" : "") + '">' + esc(stateName) + "</span>" +
        '<div class="cell-mono">' + esc(health) + (row.pid ? " · pid " + esc(row.pid) : "") + "</div></td>" +
        '<td class="cell-mono">' + esc((spec.depends_on || []).join(", ") || "—") + "</td>" +
        '<td>' + actions + "</td></tr>";
    }).join("");

    var errorRow = status.filter(function (r) { return r.last_error; }).map(function (r) {
      return '<div class="task-error"><span class="mono">' + esc(r.name) + "</span> " + esc(r.last_error) + "</div>";
    }).join("");

    var log = "";
    if (p.selected) {
      var lines = (p.log && p.log.lines) || [];
      var q = (p.logQuery || "").toLowerCase();
      if (q) lines = lines.filter(function (l) { return l.toLowerCase().indexOf(q) !== -1; });
      log = '<div class="panel process-log"><div class="panel-head"><span>Logs — ' + esc(p.selected) + "</span>" +
        '<input class="input mono process-log-filter" id="process-log-query" value="' + attr(p.logQuery || "") + '" placeholder="Filter lines">' +
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
      select +
        '<div class="panel"><div class="panel-head"><span>' + esc(plan.project || "project") + "</span>" +
        '<span class="mono panel-note">' + esc(plan.root || "") + "</span>" + approval + "</div>" +
        '<div class="panel-body flush"><table class="table process-table"><thead><tr>' +
        "<th>Process</th><th>State</th><th>Depends on</th><th></th></tr></thead><tbody>" +
        (rows || '<tr><td colspan="4" class="empty-inline">No processes declared.</td></tr>') +
        "</tbody></table></div></div>" + errorRow + log
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

  function renderShared() {
    var s = state.shared;
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

  // ------------------------------------------------------------------ projects

  function renderProjects() {
    var addForm =
      '<div class="panel"><div class="panel-body inline">' +
      '<input class="input mono grow" id="add-path" placeholder="C:\\code\\my-project">' +
      '<button type="button" class="btn btn-primary" data-action="track-path">Track it</button>' +
      '<button type="button" class="btn" data-action="init-path">Initialise a repository here</button>' +
      '<button type="button" class="btn btn-ghost" data-action="add-cancel">Cancel</button>' +
      "</div></div>";

    var body;
    if (!state.projects.length) {
      body =
        '<div class="panel"><div class="empty">' +
        '<div class="empty-title">No projects yet</div>' +
        "<div>Point devd at a repository that already exists on disk.</div>" +
        '<div class="inline" style="margin-top:8px">' +
        '<input class="input mono" id="add-path" placeholder="C:\\code\\my-project" style="width:340px">' +
        '<button type="button" class="btn btn-primary" data-action="track-path">Add project</button>' +
        '<button type="button" class="btn" data-action="init-path">Create repository here</button>' +
        "</div></div></div>";
    } else {
      body =
        (state.addOpen ? addForm : "") +
        '<div class="grid">' +
        state.projects
          .map(function (p) {
            var st = p.status;
            var stats = [];
            if (st) {
              stats.push('<span class="pill' + (st.clean ? " pill-ok" : "") + '">' +
                (st.clean ? "clean" : (st.staged.length + st.unstaged.length + st.untracked.length) + " changes") +
                "</span>");
              stats.push('<span class="pill">' + esc(st.branch || "(detached)") + "</span>");
              if (st.ahead) stats.push('<span class="pill pill-accent">' + st.ahead + " ahead</span>");
              if (st.behind) stats.push('<span class="pill pill-warn">' + st.behind + " behind</span>");
              if (st.in_progress) stats.push('<span class="pill pill-warn">' + esc(st.in_progress) + "</span>");
            } else {
              stats.push('<span class="pill pill-bad">unavailable</span>');
            }
            return (
              '<button type="button" class="card card-link" data-action="open-project" data-path="' + attr(p.path) + '">' +
              '<div class="card-title">' + esc(p.name) + "</div>" +
              '<div class="card-meta nowrap" title="' + attr(p.path) + '">' + esc(p.path) + "</div>" +
              '<div class="card-stats">' + stats.join("") + "</div>" +
              (p.error ? '<div class="row-sub" style="margin-top:6px">' + esc(p.error) + "</div>" : "") +
              "</button>"
            );
          })
          .join("") +
        "</div>";
    }

    el("main").innerHTML =
      '<div class="page">' +
      '<div class="page-head">' +
      "<div><h1 class=\"page-title\">Projects</h1>" +
      '<div class="page-sub">Every repository devd knows about, with its live state.</div></div>' +
      '<div class="page-actions">' +
      '<button type="button" class="btn" data-action="clone-form">Clone a repository</button>' +
      (state.projects.length ? '<button type="button" class="btn" data-action="add-form">Add existing</button>' : "") +
      "</div></div>" +
      body +
      "</div>";
  }

  function loadProjects() {
    return read("git.repo.list")
      .then(function (repos) {
        var list = Array.isArray(repos) ? repos : [];
        state.projects = list.map(function (p) { return { path: p.path, name: p.name, repo: p, status: null, error: null }; });
        return Promise.all(
          state.projects.map(function (p) {
            return read("git.status", { repo: p.path })
              .then(function (st) { p.status = st; })
              .catch(function (e) { p.error = e.message; });
          })
        );
      })
      .catch(function (e) {
        state.projects = [];
        toast("error", e.message);
      });
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
    recover: "Recover"
  };

  function openProject(path) {
    state.view = "project";
    state.tab = "changes";
    state.tabs = {};
    state.diff = null;
    state.project = { path: path, name: dirName(path), status: null, open: null, error: null };
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
    var code = typeof change === "string" ? "?" : change.status;
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

    var groups =
      fileGroup("Conflicted", "conflicted", st.conflicted) +
      fileGroup("Staged", "staged", st.staged) +
      fileGroup("Changed", "unstaged", st.unstaged) +
      fileGroup("Untracked", "untracked", st.untracked);

    if (!groups) groups = '<div class="empty"><div class="empty-title">Working tree clean</div></div>';

    var left =
      '<div class="panel"><div class="panel-head"><span>Working tree</span>' +
      '<span class="panel-note">' + esc(st.branch || "(detached)") + "</span>" +
      '<span class="inline right">' +
      mini("Stage all", "stage-all") +
      mini("Unstage all", "unstage-all") +
      mini("Stash changes", "stash-push") +
      "</span></div>" +
      '<div class="panel-body flush"><div class="rows">' + groups + "</div></div></div>";

    var diffPanel =
      '<div class="panel"><div class="panel-head"><span>Diff</span>' +
      (state.diff ? '<span class="panel-note">' + esc(state.diff.title) + "</span>" : "") +
      "</div>" +
      (state.diff
        ? '<div class="panel-body flush">' + renderPatch(state.diff.patch, 3000) + "</div>"
        : '<div class="empty">Select a file to see its diff</div>') +
      "</div>";

    var commit =
      '<div class="panel"><div class="panel-head"><span>Commit</span>' +
      '<span class="panel-note">' + st.staged.length + " staged</span></div>" +
      '<div class="panel-body"><div class="commit-box">' +
      '<textarea class="textarea" id="commit-message" placeholder="Commit message"></textarea>' +
      '<div class="commit-row">' +
      '<label class="check"><input type="checkbox" id="commit-amend"> Amend the last commit</label>' +
      '<button type="button" class="btn btn-primary right" data-action="commit">Commit</button>' +
      "</div></div></div></div>";

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
              var hash = e.hash || e.new_hash || "";
              var subject = e.subject || e.message || "";
              return (
                '<div class="row">' +
                '<span class="tag">' + esc(String(hash).slice(0, 7)) + "</span>" +
                '<span class="row-main nowrap">' + esc(subject) + "</span>" +
                '<span class="row-meta">' + esc(relativeTime(e.timestamp || 0)) + "</span>" +
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

  function renderProject() {
    var st = state.project.status;
    var tab = state.tab;
    var badge = "";
    if (st) {
      badge =
        '<span class="pill">' + esc(st.branch || "(detached)") + "</span>" +
        (st.clean ? '<span class="pill pill-ok">clean</span>' : '<span class="pill pill-warn">' +
          (st.staged.length + st.unstaged.length + st.untracked.length) + " changes</span>") +
        (st.ahead || st.behind ? '<span class="pill">' + st.ahead + " up / " + st.behind + " down</span>" : "") +
        (st.in_progress ? '<span class="pill pill-warn">' + esc(st.in_progress) + "</span>" : "");
    }

    var tabs = Object.keys(TAB_LABELS)
      .map(function (name) {
        return (
          '<button type="button" class="tab' + (tab === name ? " active" : "") +
          '" data-action="tab" data-tab="' + name + '">' + esc(TAB_LABELS[name]) + "</button>"
        );
      })
      .join("");

    var current = state.tabs[tab] || {};
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
    else body = "";

    el("main").innerHTML =
      '<div class="page">' +
      '<div class="page-head">' +
      "<div>" +
      '<h1 class="page-title">' + esc(state.project.name) + "</h1>" +
      '<div class="page-sub mono nowrap" title="' + attr(state.project.path) + '">' +
      esc(state.project.path) + "</div>" +
      '<div class="inline" style="margin-top:8px">' + badge + "</div>" +
      (state.project.error ? '<div class="notice notice-bad" style="margin-top:8px">' +
        esc(state.project.error) + "</div>" : "") +
      "</div>" +
      '<div class="page-actions">' +
      '<button type="button" class="btn" data-action="project-refresh">Refresh</button>' +
      '<button type="button" class="btn btn-ghost" data-action="untrack">Forget in devd</button>' +
      "</div></div>" +
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
    var detail = state.selectedCapDetail;
    var rows = state.caps.items
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
      if (detail.undo) flags.push('<span class="pill">undo: ' + esc(detail.undo) + "</span>");

      inspector =
        '<div class="panel"><div class="panel-head"><span class="mono">' + esc(detail.id) + "</span>" +
        '<span class="inline right"><span class="' + tierClass(detail.tier) + '">' + esc(detail.tier) +
        '</span><span class="pill">' + esc(detail.effect) + "</span></span></div>" +
        '<div class="panel-body stack">' +
        "<div>" + esc(detail.summary) + "</div>" +
        (detail.description ? '<div class="dim">' + esc(detail.description) + "</div>" : "") +
        '<div class="inline">' + flags.join("") + "</div>" +
        "</div></div>" +
        '<div class="panel"><div class="panel-head"><span>Arguments</span></div>' +
        '<div class="panel-body stack">' +
        '<textarea class="textarea mono" id="cap-args" spellcheck="false">' +
        esc(JSON.stringify(state.capArgs, null, 2)) + "</textarea>" +
        '<div class="inline">' +
        '<button type="button" class="btn" data-action="cap-dry-run">Dry run</button>' +
        '<button type="button" class="btn ' + (detail.requires_approval ? "btn-danger" : "btn-primary") +
        '" data-action="cap-run">Run</button>' +
        '<span class="faint">Arguments are validated by the capability, not by this form.</span>' +
        "</div></div></div>" +
        '<div class="panel"><div class="panel-head"><span>Schema</span></div>' +
        '<div class="panel-body"><pre class="patch-block">' +
        esc(JSON.stringify(detail.args_schema, null, 2)) + "</pre></div></div>";
    }

    el("main").innerHTML =
      '<div class="page">' +
      '<div class="page-head"><div><h1 class="page-title">Capabilities</h1>' +
      '<div class="page-sub">' + state.caps.total + " of " +
      ((state.session && state.session.registry.capabilities) || 0) +
      " - the same registry the CLI and the agent use.</div></div>" +
      '<div class="page-actions"><input class="input mono" id="cap-search" placeholder="Search" value="' +
      attr(state.caps.q) + '" style="width:260px"></div></div>' +
      '<div class="columns">' +
      '<div class="panel"><div class="panel-body flush"><div class="rows">' +
      (rows || '<div class="empty">No matches</div>') +
      "</div></div></div>" +
      "<div>" + inspector + "</div>" +
      "</div></div>";
  }

  function loadCapabilities() {
    var url = "/api/capabilities?limit=200&q=" + encodeURIComponent(state.caps.q) +
      (state.caps.group ? "&group=" + encodeURIComponent(state.caps.group) : "");
    return request(url).then(function (res) {
      if (!res.ok) {
        toast("error", errorOf(res.body, "capability list failed"));
        return;
      }
      state.caps.total = res.body.total;
      state.caps.items = res.body.items || [];
    });
  }

  // -------------------------------------------------------------------- activity

  function renderActivity() {
    var rows = state.audit
      .map(function (a) {
        var result = a.dry_run
          ? '<span class="pill pill-quiet">plan</span>'
          : a.ok
          ? '<span class="pill pill-ok">applied</span>'
          : '<span class="pill pill-bad">refused</span>';
        return (
          "<tr>" +
          '<td class="cell-mono">' + esc(a.at) + "</td>" +
          "<td>" + esc(a.actor) + "</td>" +
          '<td class="cell-mono">' + esc(a.capability) + "</td>" +
          '<td class="cell-mono">' + esc(a.tier) + "</td>" +
          "<td>" + result + "</td>" +
          '<td class="cell-mono">' + esc(a.args || "") + "</td>" +
          "<td>" + esc(a.message) + "</td>" +
          "</tr>"
        );
      })
      .join("");

    el("main").innerHTML =
      '<div class="page">' +
      '<div class="page-head"><div><h1 class="page-title">Activity</h1>' +
      '<div class="page-sub">Every capability attempt, successful or not, with arguments redacted. ' +
      "A <em>plan</em> row is a preview that changed nothing; a <em>refused</em> row is an operation " +
      "a human declined or that was blocked. Refusals are recorded, so a declined operation is never " +
      "indistinguishable from one that never happened.</div></div>" +
      '<div class="page-actions"><button type="button" class="btn" data-action="refresh">Refresh</button></div></div>' +
      '<div class="panel"><div class="panel-body flush">' +
      (rows
        ? '<table class="table"><thead><tr><th>When</th><th>Actor</th><th>Capability</th>' +
          "<th>Tier</th><th>Result</th><th>Arguments</th><th>Message</th></tr></thead>" +
          "<tbody>" + rows + "</tbody></table>"
        : '<div class="empty"><div class="empty-title">Nothing has happened yet</div>' +
          "<div>Audit rows appear here as soon as a capability is called from any surface.</div></div>") +
      "</div></div></div>";
  }

  function loadActivity() {
    return request("/api/audit?limit=200").then(function (res) {
      state.audit = res.ok ? res.body.items || [] : [];
    });
  }

  // ---------------------------------------------------------------------- system

  function renderSystem() {
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

    el("main").innerHTML =
      '<div class="page">' +
      '<div class="page-head"><div><h1 class="page-title">System</h1>' +
      '<div class="page-sub">The same checks `devd doctor` reports, from the same code.</div></div>' +
      '<div class="page-actions">' +
      '<button type="button" class="btn" data-action="refresh">Re-check</button>' +
      '<button type="button" class="btn btn-danger" data-action="quit">Quit devd</button>' +
      "</div></div>" +
      '<div class="panel"><div class="panel-head"><span>Environment</span>' +
      '<span class="panel-note">' + (state.doctor && state.doctor.ok ? "healthy" : "attention needed") + "</span></div>" +
      '<div class="panel-body flush">' +
      '<table class="table"><thead><tr><th>Check</th><th>Level</th><th>Detail</th></tr></thead>' +
      "<tbody>" + checkRows + "</tbody></table>" +
      "</div></div>" +
      '<div class="panel"><div class="panel-head"><span>Layout</span></div>' +
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
      '<div class="panel-body inline">' + groups + "</div></div>" +
      '<div class="panel"><div class="panel-head"><span>Not yet implemented</span></div>' +
      '<div class="panel-body stack">' +
      "<div>Services, sites, runtimes, DNS and certificates have no capabilities registered yet, so there is " +
      "nothing honest to show for them. They are the next milestones, and they will appear here as soon as " +
      "they exist rather than as disabled switches now.</div>" +
      '<div class="dim">What works today: projects, the complete Git lifecycle, the capability registry, ' +
      "the audit trail and this interface.</div>" +
      "</div></div>" +
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
      var pill = el("health-pill");
      pill.textContent = state.doctor.ok ? "healthy" : "attention needed";
      pill.className = "pill " + (state.doctor.ok ? "pill-ok" : "pill-bad");
      pill.setAttribute("data-action", "nav");
      pill.setAttribute("data-view", "system");
    });
  }

  function loadSystem() {
    return Promise.all([loadDoctor(), loadSession()]);
  }

  // ------------------------------------------------------------------- refresh

  function refresh() {
    state.tabs = state.tabs || {};
    if (SERVER_VIEWS.indexOf(state.view) !== -1) return loadServer().then(render);
    if (state.view === "projects") return loadProjects().then(render);
    if (state.view === "project") {
      state.tabs = {};
      return loadProject()
        .then(function () { return loadTab(state.tab); })
        .then(render);
    }
    if (state.view === "agents") return loadAgents().then(render);
    if (state.view === "processes") return loadProcesses().then(render);
    if (state.view === "shared") return loadShared().then(render);
    if (state.view === "capabilities") return loadCapabilities().then(render);
    if (state.view === "activity") return loadActivity().then(render);
    if (state.view === "system") return loadSystem().then(render);
    return Promise.resolve();
  }

  function navigate(view) {
    if (state.view === "agents" && view !== "agents") stopAgentTimer();
    state.view = view;
    render();
    return refresh();
  }

  function renderLoading() {
    el("main").innerHTML =
      '<div class="page"><div class="loading">Reading services, sites and certificates…</div></div>';
  }

  function render() {
    rail();
    crumbs();
    statusbar();
    if (SERVER_VIEWS.indexOf(state.view) !== -1) {
      if (!state.server.loaded) return renderLoading();
      if (state.view === "overview") renderOverview();
      else if (state.view === "sites") renderSites();
      else if (state.view === "preview") renderPreview();
      else if (state.view === "services") renderServices();
      else if (state.view === "runtimes") renderRuntimes();
      else if (state.view === "certificates") renderCertificates();
      else if (state.view === "hosts") renderHosts();
    } else if (state.view === "agents") renderAgents();
    else if (state.view === "processes") renderProcesses();
    else if (state.view === "shared") renderShared();
    else if (state.view === "projects") renderProjects();
    else if (state.view === "project") renderProject();
    else if (state.view === "capabilities") renderCapabilities();
    else if (state.view === "activity") renderActivity();
    else if (state.view === "system") renderSystem();
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

  var ACTIONS = {
    nav: function (node) { navigate(node.dataset.view); },

    refresh: function () { refresh(); },

    palette: function () { openPalette(); },

    "palette-pick": function (node) { paletteActivate(Number(node.dataset.index)); },

    "dismiss-toast": function (node) {
      var t = node.closest(".toast");
      if (t && t.parentNode) t.parentNode.removeChild(t);
    },

    "close-drawer": function () { closeDrawer(); },

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

    "start-service": function (node) { act("service.start", { name: node.dataset.name }); },

    "stop-service": function (node) { act("service.stop", { name: node.dataset.name }); },

    "restart-service": function (node) { act("service.restart", { name: node.dataset.name }); },

    "remove-service": function (node) { act("service.remove", { name: node.dataset.name }); },

    "service-logs": function (node) {
      var name = node.dataset.name;
      openDrawer("service.logs " + name, '<div class="dim">reading the log</div>');
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

    "new-site": function () {
      state.siteFormOpen = !state.siteFormOpen;
      render();
      if (state.siteFormOpen && el("site-name")) el("site-name").focus();
    },

    "create-site": function () {
      var name = readInput("site-name");
      if (!name) return toast("error", "a site needs a name");
      var host = readInput("site-host");
      var args = { name: name, host: host || null, tls: checked("site-tls"), db: checked("site-db") };
      act("site.create", args).then(function (outcome) {
        if (outcome) {
          state.siteFormOpen = false;
          render();
        }
      });
    },

    "remove-site": function (node) { act("site.remove", { name: node.dataset.name }); },

    "open-url": function (node) {
      post("/api/open-url", { url: node.dataset.url }).then(function (res) {
        if (!res.ok) toast("error", errorOf(res.body, "could not open the URL"));
      });
    },

    "install-runtime": function (node) {
      toast("info", "installing " + node.dataset.kind + " — the vendor download can take a minute");
      act("runtime.install", { kind: node.dataset.kind, version: "latest" });
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
      act("hosts.sync", {}).then(function (outcome) {
        if (outcome) render();
      });
    },

    "new-session": function (node) {
      if (node.dataset.kind) newSession(node.dataset.kind);
      else openNewSession();
    },
    "open-session": function (node) { openSession(node.dataset.id); },
    "stop-session": function (node) { stopSession(node.dataset.id); },
    "rename-session": function (node) { renameSession(node.dataset.id); },
    "save-session-label": function (node) { saveSessionLabel(node.dataset.id); },
    "dismiss-session": function (node) { dismissSession(node.dataset.id); },
    "delegate-open": function () { openDelegate(); },
    "delegate-go": function () { delegateTask(); },
    "task-wait": function (node) { waitTask(node.dataset.id); },
    "task-cancel": function (node) { cancelTask(node.dataset.id); },
    "task-result": function (node) { resultTask(node.dataset.id); },

    "process-refresh": function () { loadProcesses().then(renderProcesses); },
    "process-project": function () { },
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
          '<input class="input mono" id="clone-dest" placeholder="C:\\\\code\\\\repo"></div>' +
          '<div class="inline"><button type="button" class="btn btn-primary" data-action="clone-go">Clone</button>' +
          '<span class="faint">Credentials come from your Git credential manager; devd never stores them.</span></div>' +
          "</div>"
      );
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

    "project-refresh": function () {
      return loadProject().then(function () { return loadTab(state.tab); }).then(render);
    },

    untrack: function () {
      act("git.repo.untrack", { repo: project() }).then(function (o) {
        if (o) navigate("projects");
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
            var files = (check.at_risk || []).join("\n  ");
            var go = window.confirm(
              "Switching to " + branch + " would overwrite local changes in:\n\n  " +
              files + "\n\nThose edits are not committed. Switch anyway?"
            );
            if (!go) {
              toast("warn", "Switch to " + branch + " cancelled - local changes preserved");
              return null;
            }
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
      var to = window.prompt("Rename " + node.dataset.from + " to:", node.dataset.from);
      if (!to || to === node.dataset.from) return;
      act("git.branch.rename", { repo: project(), from: node.dataset.from, to: to });
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

    "cap-dry-run": function () {
      var args = parseArgs();
      if (!args) return;
      plan(state.selectedCap, args)
        .then(function (p) {
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
      if (state.palette.open) return closePalette();
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
  });

  document.addEventListener("input", function (event) {
    if (event.target.id === "palette-input") return paletteSearch(event.target.value);
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
    if (event.target.dataset && event.target.dataset.action === "todo-status") {
      setTodoStatus(event.target.dataset.id, event.target.dataset.rev, event.target.value);
    }
  });

  el("modal-cancel").addEventListener("click", function () { decideModal(false); });
  el("modal-confirm").addEventListener("click", function () { decideModal(true); });
  el("palette").addEventListener("click", function (event) {
    if (event.target === el("palette")) closePalette();
  });
  el("modal").addEventListener("click", function (event) {
    if (event.target === el("modal")) decideModal(false);
  });

  // ----------------------------------------------------------------------- boot

  function boot() {
    render();
    // Session first, then the server data, then the environment check. The
    // dashboard must not wait for a slow doctor run to show what is running.
    return loadSession()
      .then(loadServer)
      .then(render)
      .then(loadProjects)
      .then(render)
      .then(loadAgents)
      .then(render)
      .then(loadDoctor)
      .then(render)
      .then(function () {
        if (!state.session) return;
        var pending = state.session.pending_approvals;
        if (pending) toast("warn", pending + " operations are awaiting a decision");
      });
  }

  boot();
})();
