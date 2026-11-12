/*
 * devd-motion: the runtime every devd motion composition loads.
 *
 * A composition is a pure function of time. Give `Motion.draw` a function that
 * takes a time in seconds and puts the page into exactly the state it has at
 * that moment: no timers, no animations that run on their own, no randomness
 * that is not seeded. devd renders by asking for each frame's time in turn and
 * photographing the page, so the same composition makes the same film every
 * time, on any machine, as fast or as slowly as the machine runs.
 *
 *   Motion.config({ width: 1920, height: 1080, fps: 30, seconds: 8 });
 *   Motion.draw((t) => { title.style.opacity = Motion.progress(t, 0.2, 0.8); });
 *
 * Opened on its own in a browser the same file plays the composition in a loop,
 * with Space to pause and the arrow keys to step a frame: that is the preview.
 */
(function () {
  "use strict";

  var state = { width: 1920, height: 1080, fps: 30, seconds: 5, draw: null, time: 0 };

  function clamp(v, lo, hi) { return Math.min(hi, Math.max(lo, v)); }
  function lerp(a, b, k) { return a + (b - a) * k; }

  /* Easing: each takes 0..1 and returns 0..1. */
  var ease = {
    linear: function (k) { return k; },
    in: function (k) { return k * k * k; },
    out: function (k) { return 1 - Math.pow(1 - k, 3); },
    inOut: function (k) { return k < 0.5 ? 4 * k * k * k : 1 - Math.pow(-2 * k + 2, 3) / 2; },
    /* A critically-damped settle with a small overshoot: for things that arrive. */
    settle: function (k) { var c = 1.70158 * 0.6; return 1 + (c + 1) * Math.pow(k - 1, 3) + c * Math.pow(k - 1, 2); }
  };

  /* How far through [from, to] the time is, 0..1, eased. Before `from` it is 0 and after `to` it is 1. */
  function progress(t, from, to, easing) {
    var k = to === from ? (t >= to ? 1 : 0) : clamp((t - from) / (to - from), 0, 1);
    return (easing || ease.out)(k);
  }

  /* The same thing for several items, each starting `gap` seconds after the last. */
  function stagger(t, index, from, each, gap, easing) {
    var start = from + index * gap;
    return progress(t, start, start + each, easing);
  }

  /* The first characters of a text, typed at `cps` characters a second from `from`. */
  function typed(text, t, from, cps) {
    return text.slice(0, clamp(Math.floor((t - from) * (cps || 30)), 0, text.length));
  }

  /* A number counting from a to b over [from, to]. */
  function count(a, b, t, from, to, easing) { return Math.round(lerp(a, b, progress(t, from, to, easing))); }

  /* A small seeded generator, so "random" is the same on every render. */
  function seeded(seed) {
    var s = seed >>> 0;
    return function () {
      s = (s + 0x6d2b79f5) >>> 0;
      var x = Math.imul(s ^ (s >>> 15), 1 | s);
      x = (x + Math.imul(x ^ (x >>> 7), 61 | x)) ^ x;
      return ((x ^ (x >>> 14)) >>> 0) / 4294967296;
    };
  }

  function frames() { return Math.round(state.seconds * state.fps); }

  function size() {
    var root = document.documentElement;
    root.style.setProperty("--w", state.width + "px");
    root.style.setProperty("--h", state.height + "px");
  }

  function settled() {
    var fonts = document.fonts && document.fonts.ready ? document.fonts.ready : Promise.resolve();
    return fonts.then(function () {
      return new Promise(function (done) { requestAnimationFrame(function () { requestAnimationFrame(done); }); });
    });
  }

  /* Put the page at time t and say when it has been painted. This is what a render calls. */
  function seek(t) {
    state.time = t;
    if (state.draw) state.draw(t, Math.round(t * state.fps));
    return settled().then(function () { return { t: t, frame: Math.round(t * state.fps) }; });
  }

  var Motion = {
    config: function (options) { Object.assign(state, options || {}); size(); return Motion; },
    draw: function (fn) { state.draw = fn; seek(state.time); return Motion; },
    clamp: clamp, lerp: lerp, ease: ease, progress: progress, stagger: stagger, typed: typed, count: count, seeded: seeded,
    seek: seek,
    info: function () { return { width: state.width, height: state.height, fps: state.fps, seconds: state.seconds, frames: frames() }; }
  };
  window.Motion = Motion;
  /* The surface a render uses. */
  window.__motion = { seek: seek, info: Motion.info };

  /* Preview: play it, unless it is being rendered. */
  var rendering = /[?&]render\b/.test(location.search);
  if (!rendering) {
    var playing = true, start = null, offset = 0, loop = true;
    function tick(now) {
      if (playing) {
        if (start === null) start = now - offset * 1000;
        var t = (now - start) / 1000;
        if (t >= state.seconds) { if (loop) { start = now; t = 0; } else { t = state.seconds; playing = false; } }
        offset = t;
        if (state.draw) state.draw(t, Math.round(t * state.fps));
      }
      requestAnimationFrame(tick);
    }
    window.addEventListener("keydown", function (e) {
      if (e.code === "Space") { playing = !playing; start = null; e.preventDefault(); }
      var step = 1 / state.fps;
      if (e.code === "ArrowRight" || e.code === "ArrowLeft") {
        playing = false;
        offset = clamp(offset + (e.code === "ArrowRight" ? step : -step) * (e.shiftKey ? 10 : 1), 0, state.seconds);
        if (state.draw) state.draw(offset, Math.round(offset * state.fps));
      }
    });
    window.addEventListener("load", function () { requestAnimationFrame(tick); });
  }
})();
