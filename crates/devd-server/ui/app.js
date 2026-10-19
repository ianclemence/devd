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
    view: "projects",
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
    busy: false
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
    var items = [
      { view: "projects", label: "Projects", count: state.projects.length || null },
      { view: "capabilities", label: "Capabilities", count: (state.session && state.session.registry.capabilities) || null },
      { view: "activity", label: "Activity", count: null },
      { view: "system", label: "System", count: state.health && state.health.ok === false ? "!" : null }
    ];
    el("rail").innerHTML =
      '<div class="rail-section">Workspace</div>' +
      items
        .map(function (i) {
          return (
            '<button type="button" class="rail-item' + (state.view === i.view ? " active" : "") +
            '" data-action="nav" data-view="' + i.view + '">' +
            "<span>" + esc(i.label) + "</span>" +
            (i.count !== null && i.count !== undefined
              ? '<span class="rail-count">' + esc(i.count) + "</span>"
              : "") +
            "</button>"
          );
        })
        .join("");

    var foot = [];
    if (state.session) {
      foot.push('<div class="mono nowrap" title="' + attr(state.session.paths.root) + '">' +
        esc(state.session.paths.root) + "</div>");
    }
    foot.push('<div>viewport: ' + esc((state.session && state.session.version) || "?") + "</div>");
    el("rail-foot").innerHTML = foot.join("");
  }

  function crumbs() {
    var parts = [];
    if (state.view === "project" && state.project) {
      parts.push('<button type="button" data-action="nav" data-view="projects">Projects</button>');
      parts.push('<span class="sep">/</span>');
      parts.push("<span>" + esc(state.project.name) + "</span>");
    } else {
      var labels = { projects: "Projects", capabilities: "Capabilities", activity: "Activity", system: "System" };
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
    if (state.session) {
      bits.push(esc(state.session.registry.capabilities) + " capabilities");
      bits.push("api :" + esc(state.session.port));
    }
    if (state.pending_approvals) bits.push(state.pending_approvals + " pending");
    el("statusbar").innerHTML = bits.join("<span> </span>");
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

  function loadSystem() {
    return Promise.all([
      request("/api/doctor"),
      request("/api/session")
    ]).then(function (both) {
      if (both[0].ok) state.doctor = both[0].body;
      if (both[1].ok) {
        state.session = both[1].body;
        state.pending_approvals = both[1].body.pending_approvals || 0;
      }
      state.health = state.doctor;
      var pill = el("health-pill");
      if (state.doctor) {
        pill.textContent = state.doctor.ok ? "healthy" : "attention needed";
        pill.className = "pill " + (state.doctor.ok ? "pill-ok" : "pill-bad");
        pill.setAttribute("data-action", "nav");
        pill.setAttribute("data-view", "system");
      }
    });
  }

  // ------------------------------------------------------------------- refresh

  function refresh() {
    state.tabs = state.tabs || {};
    if (state.view === "projects") return loadProjects().then(render);
    if (state.view === "project") {
      state.tabs = {};
      return loadProject()
        .then(function () { return loadTab(state.tab); })
        .then(render);
    }
    if (state.view === "capabilities") return loadCapabilities().then(render);
    if (state.view === "activity") return loadActivity().then(render);
    if (state.view === "system") return loadSystem().then(render);
    return Promise.resolve();
  }

  function navigate(view) {
    state.view = view;
    render();
    return refresh();
  }

  function render() {
    rail();
    crumbs();
    statusbar();
    if (state.view === "projects") renderProjects();
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
    return Promise.all([loadSystem(), loadProjects()]).then(function () {
      render();
      if (!state.session) return;
      var pending = state.session.pending_approvals;
      if (pending) toast("warn", pending + " operations are awaiting a decision");
    });
  }

  boot();
})();
