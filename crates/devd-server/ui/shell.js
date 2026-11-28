/* The page's frame, as the native window draws it.
 *
 * The window is the rail, the top bar and the status strip (see `shell.css`). This builds those
 * three, once, around the screen. It reads only data the server generates from the window's own
 * code: `window.DEVD_RAIL` (the rail, from `devd_app_core::pages::RAIL`), `window.DEVD_SIZES` and
 * `window.DEVD_THEMES` (the window's size table and its palettes, from `devd_app_core`). The theme
 * is chosen with the same `theme` setting the window saves, so the page opens in the palette the
 * window is in.
 *
 * A screen is drawn by `app.js` into `#screen`; this file never draws inside it.
 */
(function () {
  "use strict";

  var Shell = {
    /* The rail entry for the screen in front. */
    current: "home",
    /* What the person is looking at, for the crumb after the screen's name. */
    selection: "",
    /* Whether Back and Forward would go anywhere. The page keeps no history of its own: the
       browser's is the history, and these ask it. */
    canBack: false,
    canForward: false,
    /* The faces of everyone at work in the room, as the window's top bar shows them. */
    faces: [],
    /* The state of the machine, what is working and what it needs, for the strip. */
    health: { tone: "neutral", headline: "Checking the machine…" },
    working: 0,
    waiting: 0,
    saving: "",
    offline: false,
    update: null,
  };

  /* ------------------------------------------------------------------ the window's theme */

  function theme() {
    var found = localStorage.getItem("devd.theme") || "sling";
    var all = window.DEVD_THEMES || {};
    return all[found] || all.sling || { id: "sling", name: "Sling", dark: true, colours: {} };
  }

  /* Write the palette into `--win-*`, which `shell.css` and the screen both read. The whole
     stylesheet becomes the window's palette, not a second one next to it. */
  function paint() {
    var t = theme();
    var root = document.documentElement;
    var colours = t.colours || {};
    root.setAttribute("data-theme", t.id);
    root.setAttribute("data-dark", t.dark ? "1" : "0");
    Object.keys(colours).forEach(function (name) {
      root.style.setProperty("--win-" + name, colours[name]);
    });
    if (t.art) root.style.setProperty("--win-art", 'url("' + t.art + '")');
    else root.style.removeProperty("--win-art");
    root.style.setProperty("--win-art-strength", String(t.art_strength || 0));
    root.style.setProperty("--win-wash-from", (t.wash && t.wash.from) || "transparent");
    root.style.setProperty("--win-wash-to", (t.wash && t.wash.to) || "transparent");
    root.style.setProperty("--win-wash-angle", ((t.wash && t.wash.angle) || 0) + "deg");
  }

  /* The window's next theme, and the page with it. */
  function nextTheme() {
    var order = Object.keys(window.DEVD_THEMES || {});
    var at = order.indexOf(theme().id);
    localStorage.setItem("devd.theme", order[(at + 1) % order.length]);
    paint();
    update({ theme: theme() });
    Shell.refresh();
  }

  function chooseTheme(id) {
    if (!window.DEVD_THEMES || !window.DEVD_THEMES[id]) return;
    localStorage.setItem("devd.theme", id);
    paint();
    update({ theme: theme() });
    Shell.refresh();
  }

  /* --------------------------------------------------------------- the three bands */

  function icon(name, size) {
    return '<svg class="icon" aria-hidden="true" style="width:' + size + "px;height:" + size +
      'px"><use href="#win-icon-' + name + '"></use></svg>';
  }

  function badgeClass(tone) {
    return { ok: "is-ok", warn: "is-warn", danger: "is-danger", info: "is-info" }[tone] || "is-neutral";
  }

  function rail() {
    var items = window.DEVD_RAIL || [];
    // The mark is the rail logo of the window, served from the same artwork.
    var out = '<img class="shell-rail-mark" src="/favicon.svg" alt="devd">';
    items.forEach(function (item, n) {
      if (n === 2 || n === 6) out += '<div class="shell-rail-divider" aria-hidden="true"></div>';
      if (item.segment === "activity") out += '<div class="shell-rail-gap"></div>';
      var current = item.segment === Shell.current || (Shell.current === "project" && item.segment === "projects");
      var badge = (Shell.badges || {})[item.segment];
      out += '<button type="button" class="shell-rail-item' + (current ? " is-current" : "") +
        '" data-shell="rail" data-segment="' + item.segment + '" data-tip="' + esc(item.label + "  ·  Ctrl+" + (n + 1)) +
        '" aria-label="' + esc(item.label) + '" aria-current="' + (current ? "page" : "false") + '">' +
        icon(item.icon, 17) +
        (badge && badge.count > 0
          ? '<span class="shell-rail-badge ' + badgeClass(badge.tone) + '">' + badge.count + "</span>"
          : "") +
        (badge && badge.also ? '<span class="shell-rail-also" style="background:' + badge.also + '"></span>' : "") +
        "</button>";
    });
    return out;
  }

  function faces() {
    return (Shell.faces || []).slice(0, 5).map(function (f) {
      return '<button type="button" class="shell-face' + (f.active ? " is-live" : "") +
        '" style="background:' + f.colour + '" data-shell="face" data-route="' + esc(f.route || "") +
        '" title="' + esc(f.label) + '">' + esc(f.initials) + "</button>";
    }).join("");
  }

  function topbar() {
    return (
      '<button type="button" class="shell-top-btn shell-top-tip" data-shell="rail-toggle" data-tip="Show or hide the rail" aria-label="Show or hide the rail">' +
        icon("sidebar", 14) + "</button>" +
      '<button type="button" class="shell-top-btn" data-shell="back" aria-label="Back"' + (Shell.canBack ? "" : " disabled") + ">" +
        icon("arrow-left", 14) + "</button>" +
      '<button type="button" class="shell-top-btn" data-shell="forward" aria-label="Forward"' + (Shell.canForward ? "" : " disabled") + ">" +
        icon("arrow-right", 14) + "</button>" +
      '<div class="shell-crumbs">' + crumbs() + "</div>" +
      '<div class="shell-grow"></div>' +
      '<div class="shell-faces">' + faces() + "</div>" +
      '<button type="button" class="shell-search" data-shell="search" aria-label="Search or run a command">' +
        icon("search", 13) + '<span class="label">Search or run a command</span><kbd><span>Ctrl</span><span>+</span><span>K</span></kbd></button>'
    );
  }

  /* "Team › Schedules › the selected thing": the area first, as its own tab says. */
  function crumbs() {
    var item = (window.DEVD_RAIL || []).filter(function (r) { return r.segment === Shell.current; })[0];
    var label = Shell.crumb || (item ? item.label : "Home");
    var out = '<span class="here">' + esc(label) + "</span>";
    // A page of an area says the area first: "Team › Schedules".
    if (Shell.parent) {
      out = '<button type="button" class="shell-top-btn" data-shell="go" data-path="' + esc(Shell.parentPath || "") +
        '" style="width:auto;padding:0 4px;color:var(--win-muted);font-size:inherit">' + esc(Shell.parent) + "</button>" +
        icon("chevron-right", 11).replace('class="icon"', 'class="icon sep"') + out;
    }
    if (Shell.selection) {
      out += icon("chevron-right", 11).replace('class="icon"', 'class="icon sep"') +
        '<span class="seal">' + esc(Shell.selection) + "</span>";
    }
    return out;
  }

  /* The area's own row of tabs, as the window draws it under the top bar ("Overview · Hosts ·
   * Certificates · Mail"): the area's screen first, then its pages. It is the page's own `subnav`,
   * emptied by the page, drawn here from `window.DEVD_AREAS`, the window's page table. */
  function subnav(area) {
    var groups = window.DEVD_AREAS || [];
    var g = groups.filter(function (a) { return a.home === area; })[0];
    if (!g || g.tabs.length < 2) return "";
    return g.tabs.map(function (t) {
      var on = t.path === Shell.path;
      return '<button type="button" class="shell-tab' + (on ? " is-current" : "") +
        '" data-shell="go" data-path="' + esc(t.path) + '"' + (on ? ' aria-current="page"' : "") + ">" +
        esc(t.label) + "</button>";
    }).join("");
  }

  function status() {
    var h = Shell.health || {};
    var out = '<div class="piece"><span class="win-dot ' + (h.tone || "neutral") + '"></span><b>' + esc(h.headline || "") + "</b></div>";
    if (Shell.working > 0) {
      out += '<div class="piece"><span class="win-dot info live"></span>' + Shell.working +
        (Shell.working === 1 ? " agent working" : " agents working") + "</div>";
    }
    if (Shell.waiting > 0) {
      out += '<div class="piece"><span class="win-dot warn"></span>' + Shell.waiting +
        (Shell.waiting === 1 ? " agent waiting for your approval" : " agents waiting for your approval") + "</div>";
    }
    if (Shell.saving) out += '<div class="piece"><span class="win-dot info live"></span><b>' + esc(Shell.saving) + "</b></div>";
    if (Shell.offline) out += '<div class="piece is-warn">Offline. Reconnecting…</div>';
    out += '<div class="piece version">' + esc("devd " + (Shell.version || "")) + "</div>";
    return out;
  }

  function esc(s) {
    return String(s == null ? "" : s).replace(/[&<>"']/g, function (c) {
      return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c];
    });
  }

  /* Draw the frame around whatever is in `#screen`. Idempotent: it is called on every screen. */
  function render() {
    var frame = document.getElementById("shell-frame");
    if (!frame) return;
    var railEl = frame.querySelector(".shell-rail");
    var top = frame.querySelector(".shell-topbar");
    var strip = frame.querySelector(".shell-status");
    if (railEl) railEl.innerHTML = rail();
    if (top) top.innerHTML = topbar();
    if (strip) strip.innerHTML = status();
    // The area's tabs sit under the top bar, inside the screen column.
    var screenEl = document.getElementById("screen");
    var row = document.getElementById("shell-subnav");
    var html = subnav(Shell.area);
    if (screenEl) {
      if (!row) {
        row = document.createElement("nav");
        row.id = "shell-subnav";
        row.className = "shell-subnav";
        row.setAttribute("aria-label", "This area");
        screenEl.parentNode.insertBefore(row, screenEl);
      }
      if (row.dataset.area !== (Shell.area || "") || row.dataset.path !== (Shell.path || "")) {
        row.dataset.area = Shell.area || "";
        row.dataset.path = Shell.path || "";
        row.innerHTML = html;
      }
      row.hidden = !html;
    }
    paint();
  }

  /* ------------------------------------------------------------------------- the actions */

  document.addEventListener("click", function (ev) {
    var n = ev.target && ev.target.closest ? ev.target.closest("[data-shell]") : null;
    if (!n) return;
    var what = n.dataset.shell;
    if (what === "rail") {
      location.assign("/" + (n.dataset.segment === "home" ? "" : n.dataset.segment));
    } else if (what === "back") {
      history.back();
    } else if (what === "forward") {
      history.forward();
    } else if (what === "search") {
      document.querySelector('[data-action="palette"]') && document.querySelector('[data-action="palette"]').click();
    } else if (what === "rail-toggle") {
      document.documentElement.toggleAttribute("data-rail-off");
    } else if (what === "face") {
      if (n.dataset.route) location.assign(n.dataset.route);
    } else if (what === "go") {
      location.assign(n.dataset.path);
    }
  });

  /* A rail entry and an area's tab, pressed: the page goes there without loading itself again.
   *
   * The page is one document that redraws its screens (`app.js`), so a press here is the same as
   * one inside the screen: it is handed to the page, which changes the screen and the address.
   * Without this the frame's own buttons fell through to nothing, and a press on the rail did not
   * move at all. */
  function go(path) {
    var page = window.DEVD_PAGE;
    if (page && typeof page.go === "function") {
      page.go(path);
      return true;
    }
    return false;
  }

  document.addEventListener("click", function (ev) {
    var node = ev.target && ev.target.closest ? ev.target.closest(".shell-rail-item, .shell-tab") : null;
    if (!node) return;
    // A rail entry that opens an area goes to that area's first tab, as a tab press does.
    var path = node.dataset.path;
    if (!path && node.dataset.segment) {
      var item = (window.DEVD_RAIL || []).filter(function (r) { return r.segment === node.dataset.segment; })[0];
      path = (item && item.area) || "/" + (node.dataset.segment === "home" ? "" : node.dataset.segment);
    }
    if (!path) return;
    ev.preventDefault();
    if (!go(path)) location.assign(path);
  }, true);

  /* The window's keys: Ctrl+1..7 go to the rail, Ctrl+K opens the search. */
  document.addEventListener("keydown", function (ev) {
    var ctrl = (ev.ctrlKey || ev.metaKey) && !ev.altKey;
    if (!ctrl) return;
    if (ev.key === "k" || ev.key === "K") {
      ev.preventDefault();
      var search = document.querySelector('[data-action="palette"]');
      if (search) search.click();
      return;
    }
    var n = parseInt(ev.key, 10);
    var items = window.DEVD_RAIL || [];
    if (n >= 1 && n <= items.length) {
      ev.preventDefault();
      var segment = items[n - 1].segment;
      location.assign("/" + (segment === "home" ? "" : segment));
    }
  });

  /* What the server and the page tell the frame about itself. */
  function update(fields) {
    Object.keys(fields || {}).forEach(function (k) { Shell[k] = fields[k]; });
    render();
  }

  /* Kept apart from `update`: the window's themes are chosen by the person, not the server. */
  /* The window's chrome, from the server's rules, and where the person has just gone. */
  async function sync() {
    try {
      var res = await fetch("/api/chrome", { headers: { Authorization: "Bearer " + token() } });
      if (res.ok) {
        var chrome = await res.json();
        Shell.badges = chrome.badges || {};
        Shell.strip = chrome.strip || Shell.strip;
        Shell.version = chrome.version || Shell.version;
        if (chrome.strip) Shell.health = { tone: chrome.strip.tone, headline: chrome.strip.headline };
        Shell.working = chrome.strip ? chrome.strip.working : Shell.working;
        Shell.waiting = chrome.strip ? chrome.strip.waiting : Shell.waiting;
        if (chrome.faces && chrome.faces.length) Shell.faces = chrome.faces;
      }
    } catch (e) { /* the frame keeps what it had */ }
    render();
  }

  function token() {
    var m = document.querySelector('meta[name="devd-token"]');
    return m ? m.content : "";
  }

  Shell.sync = sync;
  Shell.nextTheme = nextTheme;
  Shell.chooseTheme = chooseTheme;
  Shell.render = render;
  Shell.update = update;
  Shell.theme = theme;
  Shell.refresh = render;
  Shell.esc = esc;

  window.DEVD_SHELL = Shell;
  window.DEVD_THEME_NEXT = nextTheme;
  window.DEVD_THEME_CHOOSE = chooseTheme;
  /* The window's icons, in the document before anything draws: one sprite, cached by the browser. */
  async function icons() {
    if (document.getElementById("win-icons")) return;
    try {
      var res = await fetch("/icons/win.svg");
      if (!res.ok) return;
      var text = await res.text();
      if (!text || text.indexOf("<svg") < 0) return;
      var box = document.createElement("div");
      box.id = "win-icons";
      box.setAttribute("aria-hidden", "true");
      box.style.cssText = "position:absolute;width:0;height:0;overflow:hidden";
      box.innerHTML = text;
      document.body.insertBefore(box, document.body.firstChild);
    } catch (e) { /* the rail shows its tooltips and names without drawings */ }
  }

  var ICONS = icons();
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", function () {
      // Draw once the icons are in the document: a rail of empty boxes that fills in a moment
      // later reads as a fault, and a cached sprite makes that a frame or two.
      ICONS.then(render);
      sync();
      // The window's strip is about what is happening now; it is asked again on the same slow
      // beat the window reads its own records.
      setInterval(sync, 15000);
    });
  } else {
    ICONS.then(render);
    sync();
  }
})();
