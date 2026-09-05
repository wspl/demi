// Standard globals built on the native helpers. Runs once at startup as a
// script; `__tinyjs_native` is removed at the end so nothing else can
// reach it.
(() => {
  const native = globalThis.__tinyjs_native;
  delete globalThis.__tinyjs_native;
  const define = (name, value) =>
    Object.defineProperty(globalThis, name, { value, writable: true, configurable: true, enumerable: false });

  // --- console ---------------------------------------------------------
  const inspect = (value, depth, seen) => {
    switch (typeof value) {
      case "string":
        return depth === 0 ? value : JSON.stringify(value);
      case "number":
        return Object.is(value, -0) ? "-0" : String(value);
      case "bigint":
        return `${value}n`;
      case "symbol":
      case "boolean":
      case "undefined":
        return String(value);
      case "function":
        return value.name ? `[Function: ${value.name}]` : "[Function (anonymous)]";
    }
    if (value === null) return "null";
    if (value instanceof Error) return value.stack ? `${value.stack}` : `${value.name}: ${value.message}`;
    if (value instanceof Date) return isNaN(value.getTime()) ? "Invalid Date" : value.toISOString();
    if (value instanceof RegExp) return String(value);
    if (value instanceof Uint8Array) {
      const head = Array.from(value.subarray(0, 32), (b) => b.toString(16).padStart(2, "0")).join(" ");
      return `Uint8Array(${value.length}) [ ${head}${value.length > 32 ? " ..." : ""} ]`;
    }
    if (seen.has(value)) return "[Circular]";
    if (depth > 3) return Array.isArray(value) ? "[Array]" : "[Object]";
    seen.add(value);
    let text;
    if (Array.isArray(value)) {
      text = `[ ${value.map((v) => inspect(v, depth + 1, seen)).join(", ")} ]`;
    } else if (value instanceof Map) {
      text = `Map(${value.size}) { ${Array.from(value, ([k, v]) => `${inspect(k, depth + 1, seen)} => ${inspect(v, depth + 1, seen)}`).join(", ")} }`;
    } else if (value instanceof Set) {
      text = `Set(${value.size}) { ${Array.from(value, (v) => inspect(v, depth + 1, seen)).join(", ")} }`;
    } else {
      const keys = Object.keys(value);
      const body = keys.map((k) => `${/^[A-Za-z_$][\w$]*$/.test(k) ? k : JSON.stringify(k)}: ${inspect(value[k], depth + 1, seen)}`);
      const tag = value.constructor && value.constructor !== Object && value.constructor.name ? `${value.constructor.name} ` : "";
      text = body.length === 0 ? `${tag}{}` : `${tag}{ ${body.join(", ")} }`;
    }
    seen.delete(value);
    return text;
  };
  const format = (args) => {
    if (typeof args[0] === "string" && args[0].includes("%")) {
      let i = 1;
      const head = args[0].replace(/%[sdifjoO%]/g, (m) => {
        if (m === "%%") return "%";
        if (i >= args.length) return m;
        const v = args[i++];
        switch (m) {
          case "%s": return typeof v === "string" ? v : inspect(v, 1, new Set());
          case "%d": case "%i": return String(parseInt(v, 10));
          case "%f": return String(parseFloat(v));
          case "%j": return JSON.stringify(v);
          default: return inspect(v, 1, new Set());
        }
      });
      return [head, ...args.slice(i).map((v) => inspect(v, 0, new Set()))].join(" ");
    }
    return args.map((v) => inspect(v, 0, new Set())).join(" ");
  };
  const writer = (fd) => (...args) => native.print(fd, `${format(args)}\n`);
  const counts = new Map();
  const timersMap = new Map();
  define("console", {
    log: writer(1), info: writer(1), debug: writer(1),
    error: writer(2), warn: writer(2), trace: writer(2),
    dir: writer(1),
    assert(cond, ...args) { if (!cond) native.print(2, `Assertion failed${args.length ? `: ${format(args)}` : ""}\n`); },
    count(label = "default") { const n = (counts.get(label) ?? 0) + 1; counts.set(label, n); native.print(1, `${label}: ${n}\n`); },
    countReset(label = "default") { counts.delete(label); },
    time(label = "default") { timersMap.set(label, native.now()); },
    timeEnd(label = "default") { const t = timersMap.get(label); if (t !== undefined) { timersMap.delete(label); native.print(1, `${label}: ${(native.now() - t).toFixed(3)}ms\n`); } },
    timeLog(label = "default", ...args) { const t = timersMap.get(label); if (t !== undefined) native.print(1, `${label}: ${(native.now() - t).toFixed(3)}ms${args.length ? ` ${format(args)}` : ""}\n`); },
    group() {}, groupEnd() {}, table(v) { native.print(1, `${format([v])}\n`); },
  });

  // --- microtasks -------------------------------------------------------
  define("queueMicrotask", (cb) => {
    if (typeof cb !== "function") throw new TypeError("queueMicrotask requires a function");
    Promise.resolve().then(cb);
  });

  // --- text encoding ------------------------------------------------------
  const utf8Labels = new Set(["utf-8", "utf8", "unicode-1-1-utf-8"]);
  class TextEncoder {
    get encoding() { return "utf-8"; }
    encode(input = "") { return native.utf8Encode(String(input)); }
    encodeInto(source, destination) {
      const bytes = native.utf8Encode(String(source));
      const written = Math.min(bytes.length, destination.length);
      // Never split a sequence: back off to a boundary.
      let n = written;
      while (n < bytes.length && n > 0 && (bytes[n] & 0xc0) === 0x80) n--;
      destination.set(bytes.subarray(0, n));
      let read = 0;
      for (let i = 0; i < n; i++) if ((bytes[i] & 0xc0) !== 0x80) read += bytes[i] >= 0xf0 ? 2 : 1;
      return { read, written: n };
    }
  }
  class TextDecoder {
    #fatal; #ignoreBOM; #rest = null; #started = false;
    constructor(label = "utf-8", options = {}) {
      if (!utf8Labels.has(String(label).trim().toLowerCase())) throw new RangeError(`The "${label}" encoding is not supported`);
      this.#fatal = !!options.fatal;
      this.#ignoreBOM = !!options.ignoreBOM;
    }
    get encoding() { return "utf-8"; }
    get fatal() { return this.#fatal; }
    get ignoreBOM() { return this.#ignoreBOM; }
    decode(input, options = {}) {
      const stream = !!options.stream;
      let bytes = input === undefined ? new Uint8Array(0)
        : input instanceof Uint8Array ? input
        : ArrayBuffer.isView(input) ? new Uint8Array(input.buffer, input.byteOffset, input.byteLength)
        : input instanceof ArrayBuffer ? new Uint8Array(input)
        : (() => { throw new TypeError("The provided value is not of type BufferSource"); })();
      if (this.#rest) {
        const joined = new Uint8Array(this.#rest.length + bytes.length);
        joined.set(this.#rest); joined.set(bytes, this.#rest.length);
        bytes = joined; this.#rest = null;
      }
      if (!this.#started && !this.#ignoreBOM && bytes.length >= 3 && bytes[0] === 0xef && bytes[1] === 0xbb && bytes[2] === 0xbf) {
        bytes = bytes.subarray(3);
      }
      this.#started = stream;
      const [text, rest] = native.utf8Decode(bytes, this.#fatal, stream);
      this.#rest = rest;
      return text;
    }
  }
  define("TextEncoder", TextEncoder);
  define("TextDecoder", TextDecoder);
  define("btoa", (s) => native.base64Encode(native.latin1Encode(String(s))));
  define("atob", (s) => native.latin1Decode(native.base64Decode(String(s).replace(/[\t\n\f\r ]/g, ""))));

  // --- crypto / performance ------------------------------------------------
  define("crypto", {
    getRandomValues(array) {
      if (!ArrayBuffer.isView(array)) throw new TypeError("Argument must be a typed array");
      if (array.byteLength > 65536) throw new RangeError("The requested length exceeds 65536 bytes");
      native.fillRandom(new Uint8Array(array.buffer, array.byteOffset, array.byteLength));
      return array;
    },
    randomUUID: () => native.randomUUID(),
  });
  define("performance", { now: () => native.now(), timeOrigin: Date.now() - native.now() });

  // --- AbortController ---------------------------------------------------
  class AbortSignal {
    #aborted = false; #reason; #listeners = new Set(); onabort = null;
    get aborted() { return this.#aborted; }
    get reason() { return this.#reason; }
    throwIfAborted() { if (this.#aborted) throw this.#reason; }
    addEventListener(type, fn, options) {
      if (type !== "abort") return;
      const entry = { fn, once: !!(options && options.once) };
      this.#listeners.add(entry);
      if (options && options.signal) options.signal.addEventListener("abort", () => this.#listeners.delete(entry));
    }
    removeEventListener(type, fn) {
      if (type !== "abort") return;
      for (const e of this.#listeners) if (e.fn === fn) this.#listeners.delete(e);
    }
    dispatchEvent(event) {
      if (event.type !== "abort") return true;
      if (typeof this.onabort === "function") this.onabort(event);
      for (const e of [...this.#listeners]) { if (e.once) this.#listeners.delete(e); e.fn.call(this, event); }
      return true;
    }
    _abort(reason) {
      if (this.#aborted) return;
      this.#aborted = true;
      this.#reason = reason === undefined ? new DOMException("This operation was aborted", "AbortError") : reason;
      this.dispatchEvent({ type: "abort", target: this });
    }
    static abort(reason) { const c = new AbortController(); c.abort(reason); return c.signal; }
    static timeout(ms) {
      const c = new AbortController();
      setTimeout(() => c.abort(new DOMException("The operation was aborted due to timeout", "TimeoutError")), ms);
      return c.signal;
    }
    static any(signals) {
      const c = new AbortController();
      for (const s of signals) {
        if (s.aborted) { c.abort(s.reason); break; }
        s.addEventListener("abort", () => c.abort(s.reason), { once: true });
      }
      return c.signal;
    }
  }
  class AbortController {
    #signal = new AbortSignal();
    get signal() { return this.#signal; }
    abort(reason) { this.#signal._abort(reason); }
  }
  class DOMException extends Error {
    constructor(message = "", name = "Error") { super(message); this.name = name; }
  }
  define("AbortSignal", AbortSignal);
  define("AbortController", AbortController);
  define("DOMException", DOMException);

  // --- structuredClone -----------------------------------------------------
  const clone = (value, seen) => {
    if (typeof value !== "object" || value === null) {
      if (typeof value === "function" || typeof value === "symbol") throw new DOMException(`${typeof value} could not be cloned.`, "DataCloneError");
      return value;
    }
    if (seen.has(value)) return seen.get(value);
    let out;
    if (Array.isArray(value)) { out = []; seen.set(value, out); for (const v of value) out.push(clone(v, seen)); return out; }
    if (value instanceof Date) return new Date(value.getTime());
    if (value instanceof RegExp) return new RegExp(value.source, value.flags);
    if (value instanceof ArrayBuffer) return value.slice(0);
    if (ArrayBuffer.isView(value)) return new value.constructor(value);
    if (value instanceof Map) { out = new Map(); seen.set(value, out); for (const [k, v] of value) out.set(clone(k, seen), clone(v, seen)); return out; }
    if (value instanceof Set) { out = new Set(); seen.set(value, out); for (const v of value) out.add(clone(v, seen)); return out; }
    if (value instanceof Error) { out = new (globalThis[value.name] ?? Error)(value.message); out.name = value.name; if (value.stack) out.stack = value.stack; return out; }
    if (Object.getPrototypeOf(value) !== Object.prototype && Object.getPrototypeOf(value) !== null) throw new DOMException("object could not be cloned.", "DataCloneError");
    out = {}; seen.set(value, out);
    for (const k of Object.keys(value)) out[k] = clone(value[k], seen);
    return out;
  };
  define("structuredClone", (value) => clone(value, new Map()));

  // --- URL ------------------------------------------------------------------
  const encodeQuery = (s) => encodeURIComponent(s).replace(/%20/g, "+").replace(/[!'()~]/g, (c) => `%${c.charCodeAt(0).toString(16).toUpperCase()}`);
  const decodeQuery = (s) => { try { return decodeURIComponent(s.replace(/\+/g, " ")); } catch { return s; } };
  class URLSearchParams {
    #list = []; #url = null;
    constructor(init) {
      if (init instanceof URLSearchParams) this.#list = init.#list.map((p) => [...p]);
      else if (typeof init === "string") this.#parse(init);
      else if (Array.isArray(init)) for (const [k, v] of init) this.#list.push([String(k), String(v)]);
      else if (init && typeof init === "object") for (const k of Object.keys(init)) this.#list.push([k, String(init[k])]);
    }
    #parse(text) {
      this.#list = [];
      for (const part of text.replace(/^\?/, "").split("&")) {
        if (!part) continue;
        const i = part.indexOf("=");
        this.#list.push(i < 0 ? [decodeQuery(part), ""] : [decodeQuery(part.slice(0, i)), decodeQuery(part.slice(i + 1))]);
      }
    }
    _attach(url, text) { this.#url = url; this.#parse(text); }
    #update() { if (this.#url) this.#url._setSearchFromParams(this.toString()); }
    get size() { return this.#list.length; }
    append(k, v) { this.#list.push([String(k), String(v)]); this.#update(); }
    delete(k, v) { this.#list = this.#list.filter(([a, b]) => !(a === String(k) && (v === undefined || b === String(v)))); this.#update(); }
    get(k) { const p = this.#list.find(([a]) => a === String(k)); return p ? p[1] : null; }
    getAll(k) { return this.#list.filter(([a]) => a === String(k)).map(([, b]) => b); }
    has(k, v) { return this.#list.some(([a, b]) => a === String(k) && (v === undefined || b === String(v))); }
    set(k, v) {
      k = String(k); v = String(v);
      const i = this.#list.findIndex(([a]) => a === k);
      if (i < 0) this.#list.push([k, v]);
      else { this.#list[i][1] = v; this.#list = this.#list.filter(([a], j) => a !== k || j === i); }
      this.#update();
    }
    sort() { this.#list.sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)); this.#update(); }
    forEach(fn, thisArg) { for (const [k, v] of this.#list) fn.call(thisArg, v, k, this); }
    keys() { return this.#list.map(([k]) => k)[Symbol.iterator](); }
    values() { return this.#list.map(([, v]) => v)[Symbol.iterator](); }
    entries() { return this.#list.map((p) => [...p])[Symbol.iterator](); }
    [Symbol.iterator]() { return this.entries(); }
    toString() { return this.#list.map(([k, v]) => `${encodeQuery(k)}=${encodeQuery(v)}`).join("&"); }
  }
  const special = new Map([["http:", "80"], ["https:", "443"], ["ws:", "80"], ["wss:", "443"], ["ftp:", "21"], ["file:", ""]]);
  const percentPath = (s) => s.replace(/[^\x21-\x7e]|[ "<>`{}]/g, (c) => encodeURIComponent(c));
  const normalizePath = (path, isSpecial) => {
    if (!isSpecial) return path;
    const out = [];
    const segs = path.split("/");
    for (let i = 1; i < segs.length; i++) {
      const s = segs[i];
      if (s === "..") { out.pop(); if (i === segs.length - 1) out.push(""); }
      else if (s === ".") { if (i === segs.length - 1) out.push(""); }
      else out.push(s);
    }
    return `/${out.join("/")}`;
  };
  class URL {
    #protocol = ""; #username = ""; #password = ""; #hostname = ""; #port = ""; #pathname = ""; #search = ""; #hash = ""; #params;
    constructor(input, base) {
      input = String(input).trim();
      const m = /^([a-zA-Z][a-zA-Z0-9+.-]*):(.*)$/s.exec(input);
      if (m) this.#parseAbsolute(`${m[1].toLowerCase()}:`, m[2]);
      else if (base !== undefined) this.#parseRelative(input, new URL(base));
      else throw new TypeError(`Invalid URL: ${input}`);
      this.#params = new URLSearchParams();
      this.#params._attach(this, this.#search);
    }
    #parseAbsolute(protocol, rest) {
      this.#protocol = protocol;
      const isSpecial = special.has(protocol);
      let hash = "", search = "";
      const hi = rest.indexOf("#");
      if (hi >= 0) { hash = rest.slice(hi); rest = rest.slice(0, hi); }
      const qi = rest.indexOf("?");
      if (qi >= 0) { search = rest.slice(qi); rest = rest.slice(0, qi); }
      if (isSpecial) rest = rest.replace(/\\/g, "/");
      if (rest.startsWith("//")) {
        rest = rest.slice(2);
        const si = rest.indexOf("/");
        let authority = si < 0 ? rest : rest.slice(0, si);
        rest = si < 0 ? "" : rest.slice(si);
        const at = authority.lastIndexOf("@");
        if (at >= 0) {
          const cred = authority.slice(0, at);
          authority = authority.slice(at + 1);
          const ci = cred.indexOf(":");
          this.#username = ci < 0 ? cred : cred.slice(0, ci);
          this.#password = ci < 0 ? "" : cred.slice(ci + 1);
        }
        const pm = /^(\[[^\]]*\]|[^:]*)(?::(\d*))?$/.exec(authority);
        if (!pm) throw new TypeError(`Invalid URL: ${protocol}//${authority}`);
        this.#hostname = pm[1].toLowerCase();
        this.#port = pm[2] && pm[2] !== special.get(protocol) ? String(Number(pm[2])) : "";
        if (isSpecial && protocol !== "file:" && this.#hostname === "") throw new TypeError("Invalid URL: empty host");
        if (isSpecial && rest === "") rest = "/";
      } else if (isSpecial) {
        throw new TypeError(`Invalid URL: ${protocol}${rest}`);
      }
      this.#pathname = percentPath(normalizePath(rest, isSpecial));
      this.#search = search === "?" ? "" : search;
      this.#hash = hash === "#" ? "" : hash;
    }
    #parseRelative(input, base) {
      this.#protocol = base.protocol; this.#username = base.username; this.#password = base.password;
      this.#hostname = base.hostname; this.#port = base.port;
      const isSpecial = special.has(this.#protocol);
      if (isSpecial) input = input.replace(/\\/g, "/");
      if (input.startsWith("//")) { this.#parseAbsolute(this.#protocol, input); return; }
      let hash = "", search = null;
      const hi = input.indexOf("#");
      if (hi >= 0) { hash = input.slice(hi); input = input.slice(0, hi); }
      const qi = input.indexOf("?");
      if (qi >= 0) { search = input.slice(qi); input = input.slice(0, qi); }
      if (input === "") {
        this.#pathname = base.pathname;
        this.#search = search === null ? base.search : search;
      } else if (input.startsWith("/")) {
        this.#pathname = percentPath(normalizePath(input, isSpecial));
        this.#search = search ?? "";
      } else {
        const dir = base.pathname.slice(0, base.pathname.lastIndexOf("/") + 1);
        this.#pathname = percentPath(normalizePath(dir + input, isSpecial));
        this.#search = search ?? "";
      }
      if (this.#search === "?") this.#search = "";
      this.#hash = hash === "#" ? "" : hash;
    }
    _setSearchFromParams(text) { this.#search = text ? `?${text}` : ""; }
    get protocol() { return this.#protocol; }
    set protocol(v) { v = String(v).replace(/:$/, "").toLowerCase(); if (/^[a-z][a-z0-9+.-]*$/.test(v) && special.has(`${v}:`) === special.has(this.#protocol)) this.#protocol = `${v}:`; }
    get username() { return this.#username; }
    set username(v) { this.#username = encodeURIComponent(String(v)); }
    get password() { return this.#password; }
    set password(v) { this.#password = encodeURIComponent(String(v)); }
    get hostname() { return this.#hostname; }
    set hostname(v) { this.#hostname = String(v).toLowerCase(); }
    get port() { return this.#port; }
    set port(v) { v = String(v); if (v === "") this.#port = ""; else if (/^\d+$/.test(v)) this.#port = String(Number(v)) === special.get(this.#protocol) ? "" : String(Number(v)); }
    get host() { return this.#port ? `${this.#hostname}:${this.#port}` : this.#hostname; }
    set host(v) { const m = /^([^:]*)(?::(\d*))?$/.exec(String(v)); if (m) { this.hostname = m[1]; this.port = m[2] ?? ""; } }
    get origin() { return special.has(this.#protocol) && this.#protocol !== "file:" ? `${this.#protocol}//${this.host}` : "null"; }
    get pathname() { return this.#pathname; }
    set pathname(v) { v = String(v); this.#pathname = percentPath(normalizePath(v.startsWith("/") ? v : `/${v}`, special.has(this.#protocol))); }
    get search() { return this.#search; }
    set search(v) { v = String(v); this.#search = v === "" || v === "?" ? "" : v.startsWith("?") ? v : `?${v}`; this.#params._attach(this, this.#search); }
    get searchParams() { return this.#params; }
    get hash() { return this.#hash; }
    set hash(v) { v = String(v); this.#hash = v === "" || v === "#" ? "" : v.startsWith("#") ? v : `#${v}`; }
    get href() {
      const cred = this.#username ? `${this.#username}${this.#password ? `:${this.#password}` : ""}@` : "";
      const authority = this.#hostname || special.has(this.#protocol) ? `//${cred}${this.host}` : "";
      return `${this.#protocol}${authority}${this.#pathname}${this.#search}${this.#hash}`;
    }
    set href(v) { const u = new URL(v); this.#protocol = u.protocol; this.#username = u.username; this.#password = u.password; this.#hostname = u.hostname; this.#port = u.port; this.#pathname = u.pathname; this.#search = u.search; this.#hash = u.hash; this.#params._attach(this, this.#search); }
    toString() { return this.href; }
    toJSON() { return this.href; }
    static canParse(input, base) { try { new URL(input, base); return true; } catch { return false; } }
    static parse(input, base) { try { return new URL(input, base); } catch { return null; } }
  }
  define("URL", URL);
  define("URLSearchParams", URLSearchParams);
})();
