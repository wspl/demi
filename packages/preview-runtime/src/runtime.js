// The runtime of a preview document or worker (`docs/browser/preview.md` § The runtime): the
// page sees the addresses and origins it wrote, and what the browser cannot do for a preview
// origin is done here.
import { mapUrl, mapWrittenUrl, logicalUrl, startRewriter, currentRewriter, withRewriter, noteLabels, addressRole, isProxyUrl, withIntegrity, markWorker } from './url-map.js';
import { rewriteJavaScript, rewriteHtml, rewriteFunctionArguments } from './rewrite.js';
import { installDomTextRuntime } from './dom-text-runtime.js';
import { installCssRuntime } from './css-runtime.js';
import { mapModuleSpecifier } from './module-specifiers.js';
import { installLocationRuntime, propertyKey } from './location-runtime.js';
import { rewriteResourceAttribute, rewriteSrcset } from './resource-attributes.js';
import { installCookieRuntime } from './cookie-runtime.js';
import { installMessageRuntime } from './message-runtime.js';
import { installUnavailableApis } from './unavailable-runtime.js';
import { installFrameRuntime } from './frame-runtime.js';
import { installWebSocketRuntime } from './websocket-runtime.js';
import { installTabRuntime } from './tab-runtime.js';
import { recordNatives, disguiseReplacements } from './native-runtime.js';
import { rewriteCss } from './css-rewrite.js';
import { on, send } from './document-channel.js';

if (!globalThis.__proxyRuntimeInstalled) {
  Object.defineProperty(globalThis, '__proxyRuntimeInstalled', { value: true });
  // The engine or the creating realm supplies this realm's boot data first.
  const boot = globalThis.__proxyBoot;
  if (!boot) throw new Error('Preview runtime started without boot data');
  const natives = recordNatives();
  // The runtime's own globals do not show up where a page lists its window's properties.
  // The rewriter declares the two temporaries with `var`, which keeps an existing property.
  Object.defineProperty(globalThis, '__proxyBoot', { enumerable: false });
  for (const name of ['__proxyObject', '__proxyKey']) Object.defineProperty(globalThis, name, { writable: true, enumerable: false, configurable: false, value: undefined });
  const realLocation = globalThis.location;
  const isWorker = !globalThis.document;
  // The scripts that started this document (the client script, the boot data, this runtime)
  // are the preview's: the page never finds them among its own.
  const runtimeScript = isWorker ? null : document.currentScript;
  const bootScript = runtimeScript?.previousElementSibling;
  const clientScript = bootScript?.previousElementSibling;
  if (runtimeScript?.getAttribute('src') === boot.runtime && bootScript?.localName === 'script' && clientScript?.getAttribute('src') === boot.client) {
    for (const script of [clientScript, bootScript, runtimeScript]) script.remove();
  }
  // A preview tab's top frame is the one whose parent is Demi's page, the real top. The engine
  // can only infer it; here it is known.
  const topLevel = !isWorker && globalThis.parent === globalThis.top;
  // A worker from a data: URL (or a blob: URL such a worker made) has an opaque origin.
  const opaque = isWorker && new URL(boot.url).origin === 'null';
  startRewriter({ ...boot, topLevel, opaque }, entries => send({ type: 'labels', entries }));
  // Labels the Demi page met in this document's responses; the channel opens at once, so that
  // they arrive before the document reads its addresses back.
  on('labels', ({ entries }) => currentRewriter().learn(JSON.stringify(entries)));
  if (!isWorker) send({ type: 'hello' });
  // Pages replace URL with polyfills (Juejin's lacks URL.parse); the runtime keeps the browser's.
  const NativeURL = globalThis.URL;
  const originalEval = globalThis.eval;
  const nativeRequestUrl = Object.getOwnPropertyDescriptor(Request.prototype, 'url').get;
  const nativeBaseURI = globalThis.Node && Object.getOwnPropertyDescriptor(Node.prototype, 'baseURI')?.get;
  const currentLogicalUrl = () => isWorker ? boot.url : logicalUrl(realLocation.href);
  const currentBaseUrl = () => !isWorker && nativeBaseURI ? logicalUrl(nativeBaseURI.call(document)) : boot.url;
  // The logical origin of this realm; a srcdoc document belongs to its parent's origin.
  const currentOrigin = () => {
    const url = new NativeURL(currentLogicalUrl());
    return ['http:', 'https:'].includes(url.protocol) ? url.origin : new NativeURL(boot.base ?? boot.url).origin;
  };
  Object.defineProperty(globalThis, '__proxyModuleSpecifier', { value: (value, base = currentBaseUrl()) => /^\s*(data|blob):/i.test(String(value)) ? rewrittenScriptUrl(String(value).trim()) : mapModuleSpecifier(value, base) });
  if (globalThis.importScripts) {
    const importScripts = globalThis.importScripts;
    globalThis.importScripts = function(...urls) { return importScripts.apply(this, urls.map(url => mapUrl(url, currentBaseUrl()))); };
  }
  // Every computed member access a[b] passes these two (see the rewriter). Numbers,
  // strings and symbols already are property keys, and a number never names a Location
  // or Window member: those return at once, or array-heavy pages slow down a thousandfold.
  Object.defineProperty(globalThis, '__proxyPropertyKey', { value: (object, key) => object == null || typeof key === 'number' || typeof key === 'string' || typeof key === 'symbol' ? key : propertyKey(key) });
  Object.defineProperty(globalThis, '__proxyLogicalUrl', { value: logicalUrl });
  Object.defineProperty(globalThis, '__proxyRewriteJavaScript', { value: (value, filename = currentBaseUrl(), options) => typeof value === 'string' ? rewriteJavaScript(value, filename, options) : value });
  Object.defineProperty(globalThis, '__proxyEval', { value: value => originalEval(globalThis.__proxyRewriteJavaScript(value)) });

  for (const Original of [Function, Object.getPrototypeOf(async function() {}).constructor, Object.getPrototypeOf(function*() {}).constructor, Object.getPrototypeOf(async function*() {}).constructor]) {
    const Wrapped = new Proxy(Original, {
      apply(target, receiver, args) { return Reflect.apply(target, receiver, rewriteFunctionArguments(args, target.name)); },
      construct(target, args, newTarget) { return Reflect.construct(target, rewriteFunctionArguments(args, target.name), newTarget); },
    });
    Object.defineProperty(Original.prototype, 'constructor', { ...Object.getOwnPropertyDescriptor(Original.prototype, 'constructor'), value: Wrapped });
    if (Original === globalThis.Function) globalThis.Function = Wrapped;
  }
  const locationRuntime = isWorker ? null : installLocationRuntime(realLocation, currentBaseUrl, boot.ancestors ?? []);
  const locationView = locationRuntime?.view ?? {};
  for (const key of locationRuntime ? [] : ['href', 'origin', 'protocol', 'host', 'hostname', 'port', 'pathname', 'search', 'hash']) {
    Object.defineProperty(locationView, key, {
      get: () => new NativeURL(currentLogicalUrl())[key],
      set: key === 'origin' ? undefined : value => {
        const destination = new NativeURL(currentLogicalUrl());
        destination[key] = value;
        realLocation.href = mapUrl(destination, currentBaseUrl(), 'navigation');
      },
    });
  }
  if (!locationRuntime) {
    locationView.toString = () => currentLogicalUrl();
    locationView[Symbol.toPrimitive] = () => currentLogicalUrl();
  }
  Object.defineProperty(globalThis, '__proxyLocation', {
    get: () => locationView,
    set: value => { realLocation.href = mapUrl(value, currentBaseUrl(), 'navigation'); },
  });
  const locationOwner = {};
  Object.defineProperty(locationOwner, 'location', {
    get: () => locationView,
    set: value => { realLocation.href = mapUrl(value, currentBaseUrl(), 'navigation'); },
  });
  Object.defineProperty(globalThis, '__proxyLocationOwner', {
    value: object => {
      // A same-origin window of another realm (the page's own blank frame, or its parent seen
      // from there) has a runtime of its own, whose location handling fits that realm.
      if (object !== globalThis && object !== null && typeof object === 'object' && object.window === object) {
        let own;
        try {
          own = object.__proxyLocationOwner;
        } catch {
          // A cross-origin window: its location is handled here, as far as it may be used.
        }
        if (own && own !== globalThis.__proxyLocationOwner) return own(object);
      }
      if (locationRuntime) return locationRuntime.propertyOwner(object, 'location');
      if (object === globalThis) return locationOwner;
      return object;
    },
  });
  const messages = installMessageRuntime(currentOrigin);
  Object.defineProperty(globalThis, '__proxyScriptOwner', { value: object => object?.window === object ? { eval: value => object.__proxyEval(value) } : object });
  Object.defineProperty(globalThis, '__proxyPropertyOwner', { value: (object, key) => typeof key === 'number' ? object : key === 'location' ? globalThis.__proxyLocationOwner(object) : key === 'postMessage' ? messages.owner(object) : key === 'eval' ? globalThis.__proxyScriptOwner(object) : key === 'top' ? globalThis.__proxyFrameOwner(object) : locationRuntime?.propertyOwner(object, key) ?? object });
  Object.defineProperty(globalThis, '__proxyLocationMemberOwner', { value: object => locationRuntime?.propertyOwner(object, 'href') ?? object });
  Object.defineProperty(globalThis, '__proxyMessageOwner', { value: messages.owner });
  Object.defineProperty(globalThis, 'origin', { configurable: true, enumerable: true, get: currentOrigin, set: undefined });
  // In the logical top-level document, top and parent are the page itself, as in a
  // tab of its own; the real top is the Demi page. window.top cannot be redefined, so
  // the rewriter sends reads of it here.
  // The logical top is the topmost preview window: the one whose parent is Demi's page. A
  // nested frame climbs to it; windows on the way are cross-origin,
  // but their parent is readable.
  const realTop = globalThis.top;
  let logicalTop;
  const topOf = () => {
    if (topLevel) return globalThis;
    if (!logicalTop) {
      let candidate = globalThis;
      while (candidate.parent !== realTop && candidate.parent !== candidate) candidate = candidate.parent;
      logicalTop = candidate;
    }
    return logicalTop;
  };
  Object.defineProperty(globalThis, '__proxyTop', { get: topOf });
  // A destructuring pattern that reads `location` or `top` from a window or document reads
  // the logical ones; every other member is read from the object itself.
  Object.defineProperty(globalThis, '__proxyPatternSource', {
    value: object => {
      if (object === null || (typeof object !== 'object' && typeof object !== 'function')) return object;
      const locationOwner = globalThis.__proxyLocationOwner(object);
      const frameOwner = globalThis.__proxyFrameOwner(object);
      if (locationOwner === object && frameOwner === object) return object;
      return new Proxy(object, {
        get(target, key) {
          if (key === 'location' && locationOwner !== object) return locationOwner.location;
          if (key === 'top' && frameOwner !== object) return frameOwner.top;
          return Reflect.get(target, key, target);
        },
      });
    },
  });
  // Every window of the proxied frame tree has the same logical top (a cross-origin window's
  // `window` is readable, so this tells windows apart from other objects).
  Object.defineProperty(globalThis, '__proxyFrameOwner', { value: object => object !== null && typeof object === 'object' && object.window === object ? { top: topOf() } : object });
  if (topLevel) Object.defineProperty(globalThis, 'parent', { configurable: true, enumerable: true, get: () => globalThis, set: undefined });
  installUnavailableApis();

  // The Host's engine keeps the cookies; a response tells when they changed.
  const cookies = isWorker ? null : installCookieRuntime(boot, currentLogicalUrl);
  // A response that changed cookies resolves once the view shows them, as natively.
  const noteResponse = async response => {
    if (response.headers.get('x-demi-cookie-changed')) await cookies?.cookiesChanged();
    return response;
  };
  // A Request resolves its URL at construction, against the preview address.
  const NativeRequest = globalThis.Request;
  globalThis.Request = new Proxy(NativeRequest, {
    construct(target, args, newTarget) {
      if (args.length && !(args[0] instanceof NativeRequest)) args = [mapUrl(String(args[0]), currentBaseUrl()), ...args.slice(1)];
      return Reflect.construct(target, args, newTarget);
    },
  });
  // The forwarder sees the request as the page made it; only an absolute address changes.
  const originalFetch = globalThis.fetch;
  globalThis.fetch = function fetch(input, init) {
    const target = input instanceof NativeRequest ? input : mapUrl(String(input), currentBaseUrl());
    return originalFetch.call(this, target, init).then(noteResponse);
  };

  const NativeXHR = globalThis.XMLHttpRequest;
  const nativeXHROpen = NativeXHR?.prototype.open;
  const nativeXHRSend = NativeXHR?.prototype.send;
  const nativeXHROverrideMimeType = NativeXHR?.prototype.overrideMimeType;
  const nativeXHRAllHeaders = NativeXHR?.prototype.getAllResponseHeaders;
  const NativeBlob = globalThis.Blob;
  const nativeCreateObjectURL = URL.createObjectURL;

  // Read script source from a Blob or data: URL with the browser's own decoder.
  function readScriptSource(url) {
    const request = new NativeXHR();
    nativeXHROpen.call(request, 'GET', url, false);
    nativeXHROverrideMimeType.call(request, 'text/javascript;charset=utf-8');
    nativeXHRSend.call(request);
    return request.responseText;
  }

  // Script code in a data: or blob: URL is not rewritten by the engine. Read it here,
  // rewrite it, and run the result from a data: URL.
  const rewrittenScripts = new Map();
  function rewrittenScriptUrl(address) {
    if (!rewrittenScripts.has(address)) {
      const source = readScriptSource(address);
      const rewritten = rewriteJavaScript(source, address, { base: currentBaseUrl() });
      rewrittenScripts.set(address, `data:text/javascript;charset=utf-8,${encodeURIComponent(rewritten)}`);
    }
    return rewrittenScripts.get(address);
  }

  // A worker's runtime starts from the first line of its script (crates/preview-rewrite/src/boot.rs).
  // The engine adds it to a script from an address, marked as a worker's, which keeps the
  // address a shared worker is known by; a blob: or data: script, which the engine never sees,
  // gets it here and starts from a rewritten copy. A data: worker's origin is opaque.
  function workerSource(address, type) {
    const rewrite = rewriter => rewriter.rewriteWorker(readScriptSource(address), address, type === 'module', currentLogicalUrl());
    const source = new NativeURL(address).origin === 'null'
      ? withRewriter({ ...JSON.parse(currentRewriter().bootData(address, address, currentLogicalUrl())), topLevel: false, opaque: true }, rewrite)
      : rewrite(currentRewriter());
    noteLabels();
    return source;
  }
  const scriptUrl = source => nativeCreateObjectURL.call(URL, new NativeBlob([source], { type: 'text/javascript' }));
  // The options read once, as the browser reads them: the dictionary's members in order, each
  // converted to its type. A shared worker's may be its name.
  const workerMembers = [['credentials', String], ['name', String], ['type', String]];
  const optionMembers = { Worker: workerMembers, SharedWorker: [...workerMembers, ['extendedLifetime', Boolean], ['sameSiteCookies', String]] };
  function workerOptions(name, given) {
    if (given === null || typeof given !== 'object') return given;
    const options = {};
    for (const [key, convert] of optionMembers[name]) {
      const value = given[key];
      if (value !== undefined) options[key] = convert(value);
    }
    return options;
  }
  // The worker's channel to the Demi page (its WebSockets, the labels it maps): the first
  // message on the worker, or on this document's port of a shared worker.
  function sendChannel(post) {
    globalThis.__demiPreview?.open?.('document').then(post, () => {
      // No Demi page answered: the worker runs without WebSockets, as this document does.
    });
  }
  const nativeWorkerPost = globalThis.Worker?.prototype.postMessage;
  const nativeSharedPort = globalThis.SharedWorker && Object.getOwnPropertyDescriptor(SharedWorker.prototype, 'port').get;
  const nativePortPost = MessagePort.prototype.postMessage;
  const nativeRevokeObjectURL = URL.revokeObjectURL;
  // Shared workers' copies by the page's address and type, kept by the topmost window of this
  // origin, so that every frame of it that starts the same address reaches the same worker.
  const sharedCopies = new Map();
  // The copy is this window's blob: a frame that started the worker may go, and its blobs with it.
  if (!isWorker) Object.defineProperty(globalThis, '__proxySharedCopy', { value: (address, type, source) => {
    const key = `${type}\n${address}`;
    if (!sharedCopies.has(key)) sharedCopies.set(key, scriptUrl(source()));
    return sharedCopies.get(key);
  } });
  // A revoked address keeps its copy's address, revoked too: a shared worker already running
  // under it is still reached, and a new one fails to load, as natively.
  if (!isWorker) Object.defineProperty(globalThis, '__proxySharedRevoke', { value: address => {
    for (const [key, copy] of sharedCopies) {
      if (key.slice(key.indexOf('\n') + 1) === address) nativeRevokeObjectURL.call(URL, copy);
    }
  } });
  const nativeParent = !isWorker && Object.getOwnPropertyDescriptor(globalThis, 'parent').get;
  function copyOwner() {
    let owner = globalThis;
    for (;;) {
      const parent = nativeParent.call(owner);
      try {
        if (parent === owner || !parent.__proxySharedCopy) return owner;
      } catch {
        // Another origin (or the Demi page) above: this one is the topmost of its origin.
        return owner;
      }
      owner = parent;
    }
  }
  // The page's revocation of a blob: address revokes its shared worker copies too.
  URL.revokeObjectURL = function revokeObjectURL(url) {
    const result = nativeRevokeObjectURL.call(this, url);
    if (!isWorker) copyOwner().__proxySharedRevoke(String(url));
    return result;
  };
  // A WebSocket goes through the Demi page; a worker's own requests go through the forwarder.
  installWebSocketRuntime(currentBaseUrl);
  for (const name of ['EventSource', 'Worker', 'SharedWorker']) {
    const Original = globalThis[name];
    if (!Original) continue;
    globalThis[name] = new Proxy(Original, {
      construct(target, args, newTarget) {
        if (args.length === 0) return Reflect.construct(target, args, newTarget);
        const address = String(args[0]).trim();
        if (name === 'EventSource') return Reflect.construct(target, [mapUrl(address, currentBaseUrl()), ...args.slice(1)], newTarget);
        const options = workerOptions(name, args[1]);
        const type = options !== null && typeof options === 'object' ? options.type : undefined;
        // An unknown type is the browser's TypeError, before anything starts.
        if (type !== undefined && type !== 'classic' && type !== 'module') return Reflect.construct(target, [address, options], newTarget);
        const local = /^(data|blob):/i.test(address);
        let script;
        try {
          if (!local) script = markWorker(mapUrl(address, currentBaseUrl()), type === 'module');
          else if (name === 'SharedWorker') script = copyOwner().__proxySharedCopy(address, type, () => workerSource(address, type));
          else script = scriptUrl(workerSource(address, type));
        } catch {
          // A script this realm cannot read (a blob: URL another storage partition made) or
          // parse: the worker starts as the page asked, and the browser reports the failure.
          return Reflect.construct(target, [address, options], newTarget);
        }
        const worker = Reflect.construct(target, [script, options], newTarget);
        if (name === 'Worker') {
          // The browser read the copy when the worker was made.
          if (local) nativeRevokeObjectURL.call(URL, script);
          sendChannel(port => nativeWorkerPost.call(worker, { __demiChannel: 1 }, [port]));
        } else {
          sendChannel(port => nativePortPost.call(nativeSharedPort.call(worker), { __demiChannel: 1 }, [port]));
        }
        return worker;
      },
    });
  }
  if (globalThis.Worklet) {
    const addModule = Worklet.prototype.addModule;
    Worklet.prototype.addModule = function(url, options) { return addModule.call(this, mapUrl(url, currentBaseUrl()), options); };
  }
  if (globalThis.FontFace) {
    globalThis.FontFace = new Proxy(globalThis.FontFace, {
      construct(target, args, newTarget) {
        if (typeof args[1] === 'string') args = [args[0], rewriteCss(args[1], currentBaseUrl(), 'value'), ...args.slice(2)];
        return Reflect.construct(target, args, newTarget);
      },
    });
  }
  if (!isWorker) {
    installFrameRuntime(
      address => JSON.parse(currentRewriter().parsePreviewUrl(String(address)) ?? 'null')?.bootstrap === true,
      () => ({ ...JSON.parse(currentRewriter().bootData('about:blank', currentBaseUrl(), currentLogicalUrl())), cookie: document.cookie }),
    );
  }

  // Idle callbacks: Chrome gives a cross-origin iframe idle time far less often than a top-level
  // page (seconds apart on a static page, almost never during video playback), and every
  // proxied page is such a frame. A callback the browser has not run within a short wait runs
  // from a timer with a small time budget, as polyfills do; whichever comes first cancels the other.
  if (globalThis.requestIdleCallback) {
    const nativeRequest = globalThis.requestIdleCallback;
    const nativeCancel = globalThis.cancelIdleCallback;
    const fallbackWait = 50;
    const budget = 20;
    const fallbacks = new Map();
    globalThis.requestIdleCallback = function requestIdleCallback(callback, options) {
      let id;
      const run = deadline => {
        clearTimeout(fallbacks.get(id));
        fallbacks.delete(id);
        callback(deadline);
      };
      id = nativeRequest.call(this, deadline => { if (fallbacks.has(id)) run(deadline); }, options);
      fallbacks.set(id, setTimeout(() => {
        nativeCancel.call(globalThis, id);
        const started = performance.now();
        run({ didTimeout: false, timeRemaining: () => Math.max(0, started + budget - performance.now()) });
      }, fallbackWait));
      return id;
    };
    globalThis.cancelIdleCallback = function cancelIdleCallback(id) {
      clearTimeout(fallbacks.get(id));
      fallbacks.delete(id);
      return nativeCancel.call(this, id);
    };
  }
  // Media Session artwork: the browser fetches it for its media controls, under the page's
  // CSP, so it is mapped like any image; the page reads back its own addresses.
  if (globalThis.MediaMetadata) {
    const artwork = Object.getOwnPropertyDescriptor(MediaMetadata.prototype, 'artwork');
    const mapImages = images => images == null ? images : Array.from(images, image => image?.src === undefined ? image : { ...image, src: mapUrl(String(image.src), currentBaseUrl()) });
    const views = new WeakMap();
    globalThis.MediaMetadata = new Proxy(MediaMetadata, {
      construct(target, [init, ...rest], newTarget) {
        if (init?.artwork !== undefined) init = { ...init, artwork: mapImages(init.artwork) };
        return Reflect.construct(target, [init, ...rest], newTarget);
      },
    });
    Object.defineProperty(MediaMetadata.prototype, 'artwork', {
      ...artwork,
      get() {
        // The native array stays the same until artwork changes; so does its logical view.
        const images = artwork.get.call(this);
        if (!views.has(images)) views.set(images, Object.freeze(images.map(image => Object.freeze({ ...image, src: isProxyUrl(image.src) ? logicalUrl(image.src) : image.src }))));
        return views.get(images);
      },
      set(value) { artwork.set.call(this, mapImages(value)); },
    });
  }
  if (globalThis.WorkerLocation) {
    for (const key of ['href', 'origin', 'protocol', 'host', 'hostname', 'port', 'pathname', 'search', 'hash']) {
      const descriptor = Object.getOwnPropertyDescriptor(WorkerLocation.prototype, key);
      Object.defineProperty(WorkerLocation.prototype, key, {
        ...descriptor,
        get() {
          descriptor.get.call(this);
          return new NativeURL(currentLogicalUrl())[key];
        },
      });
    }
  }
  if (NativeXHR) {
    // A document's own synchronous request reaches no service worker, so it cannot reach the
    // Host: it fails as a network error does (`docs/browser/preview.md` § The preview engine). The runtime's
    // own synchronous reads of blob: and data: scripts are not the page's.
    const refusedSync = new WeakMap();
    NativeXHR.prototype.open = function(method, url, ...rest) {
      if (!isWorker && rest.length && !rest[0]) refusedSync.set(this, new NativeURL(String(url), currentBaseUrl()).href);
      else refusedSync.delete(this);
      return nativeXHROpen.call(this, method, mapUrl(url, currentBaseUrl()), ...rest);
    };
    NativeXHR.prototype.send = function(body) {
      if (refusedSync.has(this)) {
        throw new DOMException(`Failed to execute 'send' on 'XMLHttpRequest': Failed to load '${refusedSync.get(this)}'.`, 'NetworkError');
      }
      this.addEventListener('readystatechange', () => {
        // getAllResponseHeaders lists only exposed headers, without a console warning.
        if (this.readyState === NativeXHR.HEADERS_RECEIVED && /^x-demi-cookie-changed:/im.test(nativeXHRAllHeaders.call(this))) cookies?.cookiesChanged();
      });
      return nativeXHRSend.call(this, body);
    };
  }
  if (globalThis.navigator?.sendBeacon) {
    const originalBeacon = navigator.sendBeacon.bind(navigator);
    navigator.sendBeacon = (url, data) => originalBeacon(mapUrl(url, currentBaseUrl()), data);
  }

  // Performance entries name the addresses the page loaded, not the preview's; the preview's
  // own files (the client script, the runtime, the cookie endpoint) are not the page's.
  if (globalThis.PerformanceEntry) {
    const nativeName = Object.getOwnPropertyDescriptor(PerformanceEntry.prototype, 'name').get;
    const internal = entry => {
      const name = nativeName.call(entry);
      return isProxyUrl(name) && new NativeURL(name).pathname.startsWith('/__demi/');
    };
    Object.defineProperty(PerformanceEntry.prototype, 'name', {
      ...Object.getOwnPropertyDescriptor(PerformanceEntry.prototype, 'name'),
      get() {
        const name = nativeName.call(this);
        return isProxyUrl(name) ? logicalUrl(name) : name;
      },
    });
    for (const owner of [globalThis.Performance?.prototype, globalThis.PerformanceObserverEntryList?.prototype]) {
      if (!owner) continue;
      const entries = owner.getEntries;
      const byType = owner.getEntriesByType;
      owner.getEntries = function() {
        return entries.call(this).filter(entry => !internal(entry));
      };
      owner.getEntriesByType = function(type) {
        return byType.call(this, type).filter(entry => !internal(entry));
      };
      // The browser compares its own names, the preview addresses; the page asks by logical name.
      owner.getEntriesByName = function(name, type) {
        return entries.call(this).filter(entry => !internal(entry) && entry.name === String(name) && (type === undefined || entry.entryType === String(type)));
      };
    }
  }

  for (const [constructorName, property] of [['Response', 'url'], ['Request', 'url'], ['WebSocket', 'url'], ['EventSource', 'url'], ['XMLHttpRequest', 'responseURL'], ['Document', 'URL'], ['Document', 'documentURI'], ['Node', 'baseURI']]) {
    const prototype = globalThis[constructorName]?.prototype;
    if (!prototype) continue;
    const descriptor = Object.getOwnPropertyDescriptor(prototype, property);
    if (!descriptor?.get || !descriptor.configurable) continue;
    Object.defineProperty(prototype, property, {
      ...descriptor,
      get() {
        const value = descriptor.get.call(this);
        return value && isProxyUrl(value) ? logicalUrl(value) : value;
      },
    });
  }
  if (globalThis.document) {
    const domText = installDomTextRuntime(currentBaseUrl);
    installCssRuntime(currentBaseUrl);
    const referrer = Object.getOwnPropertyDescriptor(Document.prototype, 'referrer');
    Object.defineProperty(Document.prototype, 'referrer', { ...referrer, get() {
      referrer.get.call(this);
      return boot.referrer;
    } });
    const domain = Object.getOwnPropertyDescriptor(Document.prototype, 'domain');
    Object.defineProperty(Document.prototype, 'domain', {
      ...domain,
      get() { return new NativeURL(currentOrigin()).hostname; },
      // Relaxing document.domain cannot join two preview origins; the assignment is accepted
      // and changes nothing.
      set() {},
    });
    for (const [prototype, methods] of [[Node.prototype, ['appendChild', 'insertBefore', 'replaceChild']], [Element.prototype, ['append', 'prepend', 'replaceChildren', 'insertAdjacentElement', 'before', 'after', 'replaceWith']], [Document.prototype, ['append', 'prepend', 'replaceChildren']], [DocumentFragment.prototype, ['append', 'prepend', 'replaceChildren']], [Range.prototype, ['insertNode']]]) {
      for (const method of methods) {
        const original = prototype[method];
        prototype[method] = function(...args) {
          for (const argument of args) {
            domText.prepare(argument);
            domText.prepareChild(this, argument);
          }
          return original.apply(this, args);
        };
      }
    }
    for (const [prototype, property] of [[Element.prototype, 'innerHTML'], [Element.prototype, 'outerHTML'], [ShadowRoot.prototype, 'innerHTML'], [HTMLIFrameElement.prototype, 'srcdoc']]) {
      const descriptor = Object.getOwnPropertyDescriptor(prototype, property);
      Object.defineProperty(prototype, property, { ...descriptor, set(value) {
        const source = String(value);
        const text = property === 'innerHTML' && domText.textElement(this) ? domText.rewrite(this, source) : undefined;
        if (text) {
          descriptor.set.call(this, text.output);
          domText.remember(this, text);
        } else if (property === 'srcdoc') {
          // A srcdoc document is this document's origin and environment, with its cookies.
          const base = currentBaseUrl();
          const srcdocBoot = { ...JSON.parse(currentRewriter().bootData('about:srcdoc', base, base)), cookie: document.cookie };
          descriptor.set.call(this, rewriteHtml(source, base, { boot: JSON.stringify(srcdocBoot) }));
        } else {
          descriptor.set.call(this, rewriteHtml(source, currentBaseUrl(), { fragment: true }));
        }
      } });
    }
    // Markup parsed elsewhere and then moved into the page loads what it names: parsers are
    // mapped like innerHTML (an inert document gets no runtime).
    if (globalThis.DOMParser) {
      const parseFromString = DOMParser.prototype.parseFromString;
      DOMParser.prototype.parseFromString = function(source, type) {
        const html = String(type).toLowerCase() === 'text/html';
        return parseFromString.call(this, html ? rewriteHtml(String(source), currentBaseUrl()) : source, type);
      };
    }
    const contextualFragment = Range.prototype.createContextualFragment;
    Range.prototype.createContextualFragment = function(html) {
      return contextualFragment.call(this, rewriteHtml(String(html), currentBaseUrl(), { fragment: true }));
    };
    for (const prototype of [Element.prototype, ShadowRoot.prototype]) {
      const setHtml = prototype.setHTMLUnsafe;
      if (!setHtml) continue;
      prototype.setHTMLUnsafe = function(html, ...rest) {
        return setHtml.call(this, rewriteHtml(String(html), currentBaseUrl(), { fragment: true }), ...rest);
      };
    }
    if (Document.parseHTMLUnsafe) {
      const parseHtml = Document.parseHTMLUnsafe;
      Document.parseHTMLUnsafe = function(html, ...rest) {
        return parseHtml.call(this, rewriteHtml(String(html), currentBaseUrl()), ...rest);
      };
    }
    for (const method of ['write', 'writeln']) {
      const original = Document.prototype[method];
      Document.prototype[method] = function(...parts) {
        return original.call(this, rewriteHtml(parts.join(''), currentBaseUrl(), { fragment: true }));
      };
    }
    const adjacentHtml = Element.prototype.insertAdjacentHTML;
    Element.prototype.insertAdjacentHTML = function(position, html) {
      return adjacentHtml.call(this, position, rewriteHtml(String(html), currentBaseUrl(), { fragment: true }));
    };
    // Integrity is checked by the engine against the upstream bytes (see withIntegrity);
    // the browser would check the rewritten bytes. Scripts from data: and blob: URLs run rewritten.
    const integrities = new WeakMap();
    const scriptSources = new WeakMap();
    const addressAttribute = element => element instanceof HTMLScriptElement ? 'src' : element instanceof HTMLLinkElement ? 'href' : undefined;
    const nativeSetAttribute = Element.prototype.setAttribute;
    const nativeGetAttribute = Element.prototype.getAttribute;
    function elementAddress(element, attribute, text) {
      if (element instanceof HTMLScriptElement && attribute === 'src' && /^\s*(data|blob):/i.test(text)) {
        scriptSources.set(element, text);
        return rewrittenScriptUrl(text.trim());
      }
      scriptSources.delete(element);
      const integrity = attribute === addressAttribute(element) && integrities.get(element);
      // The engine checks integrity: the address carries it, so even a relative one is mapped.
      return integrity ? withIntegrity(mapUrl(text, currentBaseUrl()), integrity) : mapWrittenUrl(text, currentBaseUrl(), addressRole(element, attribute));
    }
    function setIntegrity(element, value) {
      integrities.set(element, String(value));
      const attribute = addressAttribute(element);
      const address = nativeGetAttribute.call(element, attribute);
      if (address) nativeSetAttribute.call(element, attribute, withIntegrity(mapUrl(address, currentBaseUrl()), String(value)));
    }
    for (const constructorName of ['HTMLScriptElement', 'HTMLLinkElement']) {
      const prototype = globalThis[constructorName].prototype;
      const descriptor = Object.getOwnPropertyDescriptor(prototype, 'integrity');
      Object.defineProperty(prototype, 'integrity', { ...descriptor, get() { return integrities.get(this) ?? ''; }, set(value) { setIntegrity(this, value); } });
    }
    // A form's real destination, for a POST to another preview origin (below): read before the
    // reflection that follows makes these properties give the logical address.
    const nativeAction = Object.getOwnPropertyDescriptor(HTMLFormElement.prototype, 'action').get;
    const nativeFormAction = new Map([HTMLButtonElement, HTMLInputElement].map(type => [type, Object.getOwnPropertyDescriptor(type.prototype, 'formAction').get]));
    for (const [constructorName, property] of [
      ['HTMLIFrameElement', 'src'], ['HTMLFrameElement', 'src'], ['HTMLScriptElement', 'src'], ['HTMLImageElement', 'src'],
      ['HTMLAnchorElement', 'href'], ['HTMLLinkElement', 'href'], ['HTMLFormElement', 'action'],
      ['HTMLInputElement', 'formAction'], ['HTMLButtonElement', 'formAction'],
      ['HTMLSourceElement', 'src'], ['HTMLMediaElement', 'src'], ['HTMLVideoElement', 'poster'],
      ['HTMLTrackElement', 'src'], ['HTMLObjectElement', 'data'], ['HTMLEmbedElement', 'src'],
      ['HTMLAreaElement', 'href'], ['HTMLBaseElement', 'href'],
    ]) {
      const prototype = globalThis[constructorName]?.prototype;
      const descriptor = prototype && Object.getOwnPropertyDescriptor(prototype, property);
      if (!descriptor?.set || !descriptor.configurable) continue;
      const attribute = property === 'formAction' ? 'formaction' : property;
      Object.defineProperty(prototype, property, { ...descriptor, get: descriptor.get ? function() {
        const value = descriptor.get.call(this);
        if (scriptSources.has(this) && property === 'src') return new NativeURL(scriptSources.get(this).trim(), currentBaseUrl()).href;
        return value && isProxyUrl(value) ? logicalUrl(value) : value;
      } : undefined, set(value) {
        const text = String(value);
        const mapped = text.trimStart().toLowerCase().startsWith('javascript:') ? `javascript:${rewriteJavaScript(text.slice(text.indexOf(':') + 1), 'javascript-url.js', { sourceMap: 'none' })}` : elementAddress(this, attribute, text);
        return descriptor.set.call(this, mapped);
      } });
    }
    // An <a> or <area> is a common URL parser: set href, then read host or pathname. Its URL
    // components describe the logical address, as href does; a component set changes it.
    for (const constructorName of ['HTMLAnchorElement', 'HTMLAreaElement']) {
      const prototype = globalThis[constructorName].prototype;
      const href = Object.getOwnPropertyDescriptor(prototype, 'href');
      // No URL (no href, or one that does not parse) reads as empty, as natively.
      const logicalAddress = element => NativeURL.parse(href.get.call(element));
      // The element's stringifier is its href (`String(a)`, `a.toString()`).
      Object.defineProperty(prototype, 'toString', { ...Object.getOwnPropertyDescriptor(prototype, 'toString'), value: function toString() { return href.get.call(this); } });
      for (const key of ['origin', 'protocol', 'username', 'password', 'host', 'hostname', 'port', 'pathname', 'search', 'hash']) {
        const descriptor = Object.getOwnPropertyDescriptor(prototype, key);
        Object.defineProperty(prototype, key, {
          ...descriptor,
          get() {
            descriptor.get.call(this);
            return logicalAddress(this)?.[key] ?? '';
          },
          set: descriptor.set && function(value) {
            const address = logicalAddress(this);
            if (!address) return;
            address[key] = value;
            href.set.call(this, address.href);
          },
        });
      }
    }
    for (const [constructorName, property] of [['HTMLImageElement', 'srcset'], ['HTMLSourceElement', 'srcset'], ['HTMLLinkElement', 'imageSrcset']]) {
      const prototype = globalThis[constructorName].prototype;
      const descriptor = Object.getOwnPropertyDescriptor(prototype, property);
      if (!descriptor?.set) continue;
      Object.defineProperty(prototype, property, {
        ...descriptor,
        get() { return rewriteSrcset(descriptor.get.call(this), currentBaseUrl(), { reflect: true }); },
        set(value) { descriptor.set.call(this, rewriteSrcset(value, currentBaseUrl())); },
      });
    }
    const metaHttpEquiv = Object.getOwnPropertyDescriptor(HTMLMetaElement.prototype, 'httpEquiv');
    Object.defineProperty(HTMLMetaElement.prototype, 'httpEquiv', {
      ...metaHttpEquiv,
      set(value) { metaHttpEquiv.set.call(this, rewriteResourceAttribute('http-equiv', value, currentBaseUrl())); },
    });
    for (const method of ['setAttribute', 'setAttributeNS', 'getAttribute', 'getAttributeNS']) {
      const original = Element.prototype[method];
      const namespaced = method.endsWith('NS');
      const setter = method.startsWith('set');
      Element.prototype[method] = function(...args) {
        const nameIndex = namespaced ? 1 : 0;
        const name = `${args[nameIndex]}`;
        args[nameIndex] = name;
        const resourceName = (name.startsWith('xlink:') ? name.slice(6) : name).toLowerCase();
        if (resourceName === 'integrity' && addressAttribute(this)) {
          if (setter) return setIntegrity(this, args[nameIndex + 1]);
          return integrities.has(this) ? integrities.get(this) : original.apply(this, args);
        }
        // A custom element's address attributes are its own data (see the rewriter).
        const own = this.localName.includes('-') && resourceName !== 'style';
        if (own) return original.apply(this, args);
        if (setter && resourceName === addressAttribute(this)) args[nameIndex + 1] = elementAddress(this, resourceName, String(args[nameIndex + 1]));
        else if (setter) args[nameIndex + 1] = rewriteResourceAttribute(resourceName, args[nameIndex + 1], currentBaseUrl(), { role: addressRole(this, resourceName) });
        const result = original.apply(this, args);
        if (!setter && scriptSources.has(this) && resourceName === 'src') return scriptSources.get(this);
        return !setter && result !== null ? rewriteResourceAttribute(resourceName, result, currentBaseUrl(), { reflect: true }) : result;
      };
    }
    for (const [prototype, property] of [[Attr.prototype, 'value'], [Node.prototype, 'nodeValue'], [Node.prototype, 'textContent']]) {
      const descriptor = Object.getOwnPropertyDescriptor(prototype, property);
      Object.defineProperty(prototype, property, {
        ...descriptor,
        set(value) {
          const mapped = this instanceof Attr && this.localName === 'http-equiv'
            ? rewriteResourceAttribute('http-equiv', value, currentBaseUrl()) : value;
          descriptor.set.call(this, mapped);
        },
      });
    }
    for (const [prototype, methods] of [[Element.prototype, ['setAttributeNode', 'setAttributeNodeNS']], [NamedNodeMap.prototype, ['setNamedItem', 'setNamedItemNS']]]) {
      for (const method of methods) {
        const original = prototype[method];
        prototype[method] = function(attribute) {
          if (attribute instanceof Attr && attribute.localName === 'http-equiv') {
            attribute.value = rewriteResourceAttribute('http-equiv', attribute.value, currentBaseUrl());
          }
          return original.call(this, attribute);
        };
      }
    }
    installTabRuntime({ currentBaseUrl, currentLogicalUrl, topLevel, bootPath: boot.boot });
    // A POST to another preview origin cannot go through that origin's bootstrap as a
    // navigation: the Demi page keeps the request, and the bootstrap announces its token
    // (`docs/browser/preview.md` § Opening and navigating).
    const nativeSubmit = HTMLFormElement.prototype.submit;
    function crossOriginPost(form, submitter) {
      const method = (submitter?.getAttribute('formmethod') ?? form.getAttribute('method') ?? 'get').toLowerCase();
      const action = new NativeURL(submitter?.hasAttribute('formaction')
        ? nativeFormAction.get(submitter instanceof HTMLInputElement ? HTMLInputElement : HTMLButtonElement).call(submitter)
        : nativeAction.call(form));
      if (method !== 'post' || action.origin === realLocation.origin) return false;
      const parsed = JSON.parse(currentRewriter().parsePreviewUrl(action.href) ?? 'null');
      if (!parsed) return false;
      const target = new NativeURL(parsed.target, action.origin);
      const type = (submitter?.getAttribute('formenctype') ?? form.getAttribute('enctype') ?? 'application/x-www-form-urlencoded').toLowerCase();
      const data = new FormData(form, submitter ?? undefined);
      const encoded = type === 'multipart/form-data' ? new Response(data) : type === 'text/plain'
        ? new Response([...data].map(([name, value]) => `${name}=${value}`).join('\r\n'), { headers: { 'content-type': 'text/plain' } })
        : new Response(new URLSearchParams(data));
      encoded.arrayBuffer().then(body => globalThis.__demiPreview.navigateKept(target, {
        method: 'POST', url: target.href, contentType: encoded.headers.get('content-type'), body,
      }, window));
      return true;
    }
    window.addEventListener('submit', event => {
      if (!event.defaultPrevented && crossOriginPost(event.target, event.submitter)) event.preventDefault();
    });
    HTMLFormElement.prototype.submit = function submit() {
      if (!crossOriginPost(this, null)) nativeSubmit.call(this);
    };
    for (const method of ['pushState', 'replaceState']) {
      const original = history[method].bind(history);
      history[method] = (state, title, url) => original(state, title, url === undefined || url === null ? url : mapUrl(url, currentBaseUrl()));
    }
  }
  // Last: every replacement made above looks like the browser function it replaced.
  disguiseReplacements(natives);
}
