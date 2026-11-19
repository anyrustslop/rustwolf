// RustWolf fingerprint protection (MPL-2.0). Runs at the start of every page, before the
// page's own scripts. Makes RustWolf report what Tor Browser / Mullvad Browser report on
// Windows. CPU count, language, Intl locale and timezone are set inside the engine instead
// (see guard.rs), so they also hold in iframes and workers.
(() => {
  'use strict';
  const SEED = __SEED__;

  // Replaced functions present themselves as native code, like the originals: fingerprinting
  // scripts compare Function.prototype.toString to spot spoofing.
  const nativeText = new WeakMap();
  const fts = Function.prototype.toString;
  const native = (fn, name) => { nativeText.set(fn, 'function ' + name + '() {\n    [native code]\n}'); return fn; };
  const toStr = native(function toString() { return nativeText.has(this) ? nativeText.get(this) : fts.call(this); }, 'toString');

  const done = new WeakSet();
  function spoof(win) {
    try { if (!win || done.has(win) || !win.Navigator) return; } catch (e) { return; } // cross-origin: has its own copy
    done.add(win);
    try { win.Function.prototype.toString = toStr; } catch (e) {}
    const def = (obj, key, get) => {
      if (!obj) return;
      try { Object.defineProperty(obj, key, { get: native(get, 'get ' + key), set: undefined, configurable: true, enumerable: true }); } catch (e) {}
    };
    const wrap = (obj, key, make) => {
      if (!obj || typeof obj[key] !== 'function') return;
      try { obj[key] = native(make(obj[key]), key); } catch (e) {}
    };
    const N = win.Navigator.prototype;

    // Identity: exactly what Tor Browser / Mullvad Browser report on Windows.
    // (CPU count, language, Intl locale and timezone are set inside the engine itself.)
    try { delete N.deviceMemory; delete N.getBattery; } catch (e) {}
    const values = {
      oscpu: 'Windows NT 10.0; Win64; x64', buildID: '20181001000000', productSub: '20100101',
      appVersion: '5.0 (Windows)', language: 'en-US', platform: 'Win32', vendor: '', maxTouchPoints: 10,
      globalPrivacyControl: true, doNotTrack: 'unspecified', webdriver: false, pdfViewerEnabled: true,
    };
    for (const k in values) { const v = values[k]; def(N, k, function () { return v; }); }
    const langs = Object.freeze(['en-US', 'en']);
    def(N, 'languages', function () { return langs; });

    // Firefox's built-in PDF viewer: 5 plugins, 2 MIME types.
    try {
      const make = (proto, props) => Object.create(proto || Object.prototype, props);
      const mime = (type) => make(win.MimeType && win.MimeType.prototype, {
        type: { value: type, enumerable: true }, suffixes: { value: 'pdf', enumerable: true },
        description: { value: 'Portable Document Format', enumerable: true },
      });
      const mimes = [mime('application/pdf'), mime('text/pdf')];
      const list = (proto, items, key) => {
        const o = make(proto, {});
        items.forEach((it, i) => Object.defineProperty(o, i, { value: it, enumerable: true }));
        Object.defineProperties(o, {
          length: { value: items.length },
          item: { value: native(function item(i) { return items[i] || null; }, 'item') },
          namedItem: { value: native(function namedItem(n) { return items.find((it) => it[key] === n) || null; }, 'namedItem') },
          refresh: { value: native(function refresh() {}, 'refresh') },
          [Symbol.iterator]: { value: function* () { yield* items; } },
        });
        return o;
      };
      const plugins = ['PDF Viewer', 'Chrome PDF Viewer', 'Chromium PDF Viewer', 'Microsoft Edge PDF Viewer', 'WebKit built-in PDF']
        .map((name) => {
          const p = list(win.Plugin && win.Plugin.prototype, mimes, 'type');
          Object.defineProperties(p, {
            name: { value: name, enumerable: true }, filename: { value: 'internal-pdf-viewer', enumerable: true },
            description: { value: 'Portable Document Format', enumerable: true },
          });
          return p;
        });
      mimes.forEach((m) => Object.defineProperty(m, 'enabledPlugin', { value: plugins[0], enumerable: true }));
      const pa = list(win.PluginArray && win.PluginArray.prototype, plugins, 'name');
      const ma = list(win.MimeTypeArray && win.MimeTypeArray.prototype, mimes, 'type');
      def(N, 'plugins', function () { return pa; });
      def(N, 'mimeTypes', function () { return ma; });
    } catch (e) {}

    // Screen = the (top) window's size, pixel ratio 2, like Tor Browser's letterboxing.
    const sizeOf = () => { try { const t = win.top; return [t.innerWidth, t.innerHeight]; } catch (e) { return [win.innerWidth, win.innerHeight]; } };
    if (win.Screen) {
      const S = win.Screen.prototype;
      def(S, 'width', function () { return sizeOf()[0]; });
      def(S, 'height', function () { return sizeOf()[1]; });
      def(S, 'availWidth', function () { return sizeOf()[0]; });
      def(S, 'availHeight', function () { return sizeOf()[1]; });
      def(S, 'availLeft', function () { return 0; });
      def(S, 'availTop', function () { return 0; });
      def(S, 'colorDepth', function () { return 24; });
      def(S, 'pixelDepth', function () { return 24; });
    }
    def(win, 'devicePixelRatio', function () { return 2; });
    def(win, 'mozInnerScreenX', function () { return 0; });
    def(win, 'mozInnerScreenY', function () { return 0; });
    wrap(win, 'matchMedia', (mm) => function matchMedia(q) {
      const m = /^\s*\(\s*(min-|max-)?(resolution|-webkit-(min-|max-)?device-pixel-ratio|-moz-device-pixel-ratio)\s*:\s*([\d.]+)\s*(dppx|dpi|dpcm|x)?\s*\)\s*$/i.exec(String(q));
      if (!m) return mm.call(this, q);
      const kind = (m[1] || m[3] || '').toLowerCase();
      let v = parseFloat(m[4]);
      const unit = (m[5] || '').toLowerCase();
      if (unit === 'dpi') v /= 96; else if (unit === 'dpcm') v = v * 2.54 / 96;
      const matches = kind === 'min-' ? 2 >= v : kind === 'max-' ? 2 <= v : Math.abs(2 - v) < 1e-6;
      const noop = native(function addListener() {}, 'addListener');
      return Object.create(win.MediaQueryList ? win.MediaQueryList.prototype : Object.prototype, {
        matches: { value: matches, enumerable: true }, media: { value: String(q), enumerable: true },
        onchange: { value: null, writable: true }, addListener: { value: noop }, removeListener: { value: noop },
        addEventListener: { value: noop }, removeEventListener: { value: noop }, dispatchEvent: { value: () => true },
      });
    });

    // Firefox-only CSS bits.
    try {
      if (win.CSSStyleDeclaration && !('MozAppearance' in win.CSSStyleDeclaration.prototype)) {
        Object.defineProperty(win.CSSStyleDeclaration.prototype, 'MozAppearance', {
          get: native(function () { return this.getPropertyValue('appearance'); }, 'get MozAppearance'),
          set: native(function (v) { this.setProperty('appearance', v); }, 'set MozAppearance'),
          configurable: true, enumerable: true,
        });
      }
      if (win.CSS) wrap(win.CSS, 'supports', (sup) => function supports(...a) {
        const r = sup.apply(this, a);
        if (r || !/-moz-appearance/i.test(String(a[0]))) return r;
        return sup.apply(this, a.map((x) => String(x).replace(/-moz-appearance/ig, 'appearance'))) || true;
      });
    } catch (e) {}

    // Canvas: tiny invisible noise (different every session) on read-backs.
    const noise = (data) => {
      let s = SEED;
      for (let i = 0; i < data.length; i += 4) {
        s = (s * 1103515245 + 12345) >>> 0;
        if ((s & 31) === 0) { data[i] ^= (s >>> 8) & 1; data[i + 1] ^= (s >>> 9) & 1; }
      }
    };
    if (win.CanvasRenderingContext2D) {
      wrap(win.CanvasRenderingContext2D.prototype, 'getImageData', (gid) => function getImageData(...a) {
        const img = gid.apply(this, a);
        try { noise(img.data); } catch (e) {}
        return img;
      });
    }
    if (win.HTMLCanvasElement) {
      const noisy = (canvas) => {
        try {
          const c = win.document.createElement('canvas');
          c.width = canvas.width; c.height = canvas.height;
          const x = c.getContext('2d');
          x.drawImage(canvas, 0, 0);
          if (c.width && c.height) x.putImageData(x.getImageData(0, 0, c.width, c.height), 0, 0);
          return c;
        } catch (e) { return canvas; }
      };
      wrap(win.HTMLCanvasElement.prototype, 'toDataURL', (f) => function toDataURL(...a) { return f.apply(noisy(this), a); });
      wrap(win.HTMLCanvasElement.prototype, 'toBlob', (f) => function toBlob(...a) { return f.apply(noisy(this), a); });
    }

    // Timezone name (the engine already runs on UTC+0, like Tor Browser).
    if (win.Intl && win.Intl.DateTimeFormat) {
      wrap(win.Intl.DateTimeFormat.prototype, 'resolvedOptions', (ro) => function resolvedOptions() {
        const o = ro.call(this);
        if (o.timeZone === 'UTC' || o.timeZone === 'Etc/UTC') o.timeZone = 'Atlantic/Reykjavik';
        return o;
      });
    }

    // Coarse timers (1/60 s, like Firefox with resist-fingerprinting).
    const STEP = 1000 / 60;
    const coarse = (t) => Math.floor(t / STEP) * STEP;
    if (win.Performance) wrap(win.Performance.prototype, 'now', (f) => function now() { return coarse(f.call(this)); });
    wrap(win.Date, 'now', (f) => function now() { return Math.floor(coarse(f())); });
    try {
      const ts = Object.getOwnPropertyDescriptor(win.Event.prototype, 'timeStamp');
      if (ts && ts.get) def(win.Event.prototype, 'timeStamp', function () { return coarse(ts.get.call(this)); });
    } catch (e) {}

    // Same-origin frames get the same treatment the moment a page can reach them.
    const reach = (w) => { try { spoof(w); } catch (e) {} };
    const frameWindows = (node) => {
      try {
        if (node && node.tagName && /^i?frame$/i.test(node.tagName)) reach(node.contentWindow);
        if (node && node.querySelectorAll) node.querySelectorAll('iframe,frame').forEach((f) => reach(f.contentWindow));
      } catch (e) {}
    };
    for (const cls of ['HTMLIFrameElement', 'HTMLFrameElement']) {
      const P = win[cls] && win[cls].prototype;
      if (!P) continue;
      for (const key of ['contentWindow', 'contentDocument']) {
        const d = Object.getOwnPropertyDescriptor(P, key);
        if (!d || !d.get) continue;
        const g = d.get;
        def(P, key, function () { const r = g.call(this); reach(key === 'contentWindow' ? r : r && r.defaultView); return r; });
      }
    }
    if (win.Node) {
      for (const m of ['appendChild', 'insertBefore', 'replaceChild']) {
        wrap(win.Node.prototype, m, (f) => ({ [m](...a) { const r = f.apply(this, a); frameWindows(a[0]); return r; } })[m]);
      }
    }
    if (win.Element) {
      for (const m of ['append', 'prepend', 'after', 'before', 'replaceWith', 'insertAdjacentElement']) {
        wrap(win.Element.prototype, m, (f) => ({ [m](...a) { const r = f.apply(this, a); a.forEach(frameWindows); return r; } })[m]);
      }
      for (const key of ['innerHTML', 'outerHTML']) {
        const d = Object.getOwnPropertyDescriptor(win.Element.prototype, key);
        if (!d || !d.set) continue;
        const s = d.set;
        try {
          Object.defineProperty(win.Element.prototype, key, {
            get: d.get, configurable: true, enumerable: d.enumerable,
            set: native(function (v) { s.call(this, v); frameWindows(this.parentNode || this); }, 'set ' + key),
          });
        } catch (e) {}
      }
      wrap(win.Element.prototype, 'insertAdjacentHTML', (f) => function insertAdjacentHTML(...a) { const r = f.apply(this, a); frameWindows(this.parentNode || this); return r; });
    }
    for (let i = 0; i < win.length; i++) reach(win[i]);
    wrap(win, 'open', (f) => function open(...a) { const w = f.apply(this, a); reach(w); return w; });
  }

  Function.prototype.toString = toStr;
  spoof(window);
})();
