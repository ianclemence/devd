/* devd-inspector:1 */
/*
 * Lets devd's Preview tab see what happens inside the page it is showing: the errors the
 * page reports, and the element you point at. It does nothing anywhere else.
 *
 *   - It runs only when the page is inside another page (devd's preview frame) and the
 *     address is a local one (.test, .localhost, localhost, 127.0.0.1).
 *   - It sends messages to the page that framed it, and listens to nothing else.
 *   - It never changes the page, except for an outline while you are choosing an element.
 *
 * Add one line to your layout while you develop, and take it out (or leave it: it is inert
 * outside the preview) before you ship:
 *
 *     <script src="/devd-inspector.js" defer></script>
 */
(function () {
  "use strict";
  if (window.parent === window) return;
  if (!/(\.test|\.localhost|^localhost|^127\.0\.0\.1)$/.test(location.hostname)) return;
  if (window.__devdInspector) return;
  window.__devdInspector = 1;

  function send(message) {
    message.devd = 1;
    try { window.parent.postMessage(message, "*"); } catch (e) { /* the frame is gone */ }
  }
  function text(v) {
    try {
      if (v instanceof Error) return (v.stack || v.message || String(v)).slice(0, 1200);
      if (typeof v === "string") return v.slice(0, 1200);
      return JSON.stringify(v).slice(0, 1200);
    } catch (e) { return String(v).slice(0, 1200); }
  }

  // ---- what the page reports
  var seen = 0;
  function report(level, message, source, line) {
    if (seen++ > 200) return;
    send({ kind: "console", level: level, message: message, source: source || "", line: line || 0, at: Date.now() });
  }
  var errorLog = console.error;
  console.error = function () {
    report("error", Array.prototype.map.call(arguments, text).join(" "));
    return errorLog.apply(console, arguments);
  };
  window.addEventListener("error", function (e) {
    if (e.target && e.target !== window && (e.target.src || e.target.href)) {
      report("resource", "could not load " + (e.target.src || e.target.href), "", 0);
    } else {
      report("error", e.message || "script error", e.filename, e.lineno);
    }
  }, true);
  window.addEventListener("unhandledrejection", function (e) {
    report("error", "unhandled promise rejection: " + text(e.reason));
  });
  if (window.fetch) {
    var realFetch = window.fetch;
    window.fetch = function () {
      var asked = arguments[0] && arguments[0].url ? arguments[0].url : String(arguments[0]);
      return realFetch.apply(this, arguments).then(function (res) {
        if (res.status >= 400) report("request", res.status + " " + asked);
        return res;
      }, function (err) {
        report("request", "failed " + asked + ": " + text(err));
        throw err;
      });
    };
  }

  // ---- choosing an element
  var box = null;
  var picking = false;

  function selectorOf(el) {
    var parts = [];
    while (el && el.nodeType === 1 && parts.length < 6) {
      if (el.id) { parts.unshift("#" + el.id); break; }
      var part = el.tagName.toLowerCase();
      var classes = (el.className && typeof el.className === "string" ? el.className.trim().split(/\s+/) : []).filter(Boolean).slice(0, 2);
      if (classes.length) part += "." + classes.join(".");
      var parent = el.parentElement;
      if (parent) {
        var same = Array.prototype.filter.call(parent.children, function (c) { return c.tagName === el.tagName; });
        if (same.length > 1) part += ":nth-of-type(" + (same.indexOf(el) + 1) + ")";
      }
      parts.unshift(part);
      el = parent;
    }
    return parts.join(" > ");
  }

  function describe(el) {
    var cs = getComputedStyle(el);
    var rect = el.getBoundingClientRect();
    var attrs = {};
    ["href", "src", "alt", "type", "name", "placeholder", "role", "aria-label"].forEach(function (a) {
      if (el.getAttribute(a)) attrs[a] = el.getAttribute(a).slice(0, 200);
    });
    return {
      kind: "picked",
      selector: selectorOf(el),
      tag: el.tagName.toLowerCase(),
      id: el.id || "",
      classes: typeof el.className === "string" ? el.className : "",
      text: (el.innerText || el.textContent || "").trim().slice(0, 300),
      html: el.outerHTML.slice(0, 1500),
      attrs: attrs,
      rect: { x: Math.round(rect.left), y: Math.round(rect.top), w: Math.round(rect.width), h: Math.round(rect.height) },
      styles: {
        color: cs.color, background: cs.backgroundColor, font: cs.fontSize + " " + cs.fontWeight + " " + cs.fontFamily.split(",")[0],
        display: cs.display, position: cs.position, margin: cs.margin, padding: cs.padding, border: cs.border
      },
      page: location.pathname + location.search,
      title: document.title
    };
  }

  function outline(el) {
    if (!box) {
      box = document.createElement("div");
      box.setAttribute("data-devd-inspector", "");
      box.style.cssText = "position:fixed;z-index:2147483647;pointer-events:none;border:2px solid #6366f1;background:rgba(99,102,241,.12);border-radius:3px;transition:all 60ms";
      document.documentElement.appendChild(box);
    }
    var r = el.getBoundingClientRect();
    box.style.left = r.left + "px"; box.style.top = r.top + "px";
    box.style.width = r.width + "px"; box.style.height = r.height + "px";
  }

  function over(e) { if (e.target !== box) outline(e.target); }
  function swallow(e) { e.preventDefault(); e.stopPropagation(); }
  function choose(e) {
    swallow(e);
    var message = describe(e.target);
    stop();
    send(message);
  }
  function start() {
    if (picking) return;
    picking = true;
    document.addEventListener("mouseover", over, true);
    document.addEventListener("click", choose, true);
    document.addEventListener("mousedown", swallow, true);
    document.addEventListener("mouseup", swallow, true);
    document.documentElement.style.cursor = "crosshair";
  }
  function stop() {
    picking = false;
    document.removeEventListener("mouseover", over, true);
    document.removeEventListener("click", choose, true);
    document.removeEventListener("mousedown", swallow, true);
    document.removeEventListener("mouseup", swallow, true);
    document.documentElement.style.cursor = "";
    if (box && box.parentNode) box.parentNode.removeChild(box);
    box = null;
  }

  window.addEventListener("message", function (e) {
    if (e.source !== window.parent || !e.data || e.data.devd !== 1) return;
    if (e.data.kind === "pick-start") start();
    if (e.data.kind === "pick-stop") stop();
  });

  send({ kind: "hello", version: 1, url: location.href, title: document.title });
})();
