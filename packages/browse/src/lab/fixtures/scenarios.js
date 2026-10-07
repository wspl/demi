add('url-business-json-untouched', ['URL-02', 'SCRIPT-01'], () => {
  const original = { signature: 'a%2Fb+%20~', callback: `${AUTH}/callback?state=%2F&state=%2f`, content: 'User text http://localhost:19401/path' };
  return JSON.parse(JSON.stringify(original));
});
add('url-base-element', ['URL-02'], async () => {
  const base = document.createElement('base');
  base.href = `${APP}/lab/deep/`;
  document.head.prepend(base);
  try {
    const link = document.createElement('a');
    link.href = '../echo?raw=%2f+%20~';
    return { base: document.baseURI, link: link.href };
  } finally { base.remove(); }
});
add('script-dynamic-dom-srcset', ['SCRIPT-01', 'ESCAPE-01'], async () => {
  const image = document.createElement('img');
  image.srcset = `${OTHER}/lab/pixel.svg 1x`;
  document.body.append(image);
  try { await image.decode(); return { width: image.naturalWidth }; }
  finally { image.remove(); }
});
add('script-shadow-dom-html', ['SCRIPT-02', 'ESCAPE-01'], async () => {
  const host = document.createElement('div');
  document.body.append(host);
  try {
    const root = host.attachShadow({ mode: 'open' });
    root.innerHTML = `<img src="${OTHER}/lab/pixel.svg">`;
    const image = root.querySelector('img');
    await image.decode();
    return image.naturalWidth;
  } finally { host.remove(); }
});
add('script-cssom-insert-rule', ['SCRIPT-02', 'ESCAPE-01'], async () => {
  const style = document.createElement('style');
  const element = document.createElement('div');
  element.id = 'cssom';
  element.style.cssText = 'width:10px;height:10px';
  document.head.append(style);
  document.body.append(element);
  try {
    style.sheet.insertRule(`#cssom{background-image:url('${OTHER}/lab/pixel.svg')}`);
    await sleep(100);
    return getComputedStyle(element).backgroundImage;
  } finally { style.remove(); element.remove(); }
});
add('script-import-map', ['SCRIPT-02'], async () => {
  const frame = document.createElement('iframe');
  try {
    frame.srcdoc = `<script type="importmap">{"imports":{"fixture-value":"${APP}/lab/value.js"}}<\/script><script type="module">import {value} from 'fixture-value';parent.postMessage({importValue:value},'*');<\/script>`;
    let listener;
    try {
      return await deadline(new Promise(resolve => {
        listener = event => { if (event.source === frame.contentWindow && event.data?.importValue) resolve(event.data.importValue); };
        window.addEventListener('message', listener);
        document.body.append(frame);
      }));
    } finally { window.removeEventListener('message', listener); }
  } finally { frame.remove(); }
});
add('script-computed-runtime-key', ['NAV-02', 'ESCAPE-02'], () => arrival(frame => {
  const key = ['l', 'o', 'c', 'a', 't', 'i', 'o', 'n'].join('');
  frame.contentWindow[key].href = `${AUTH}/lab/arrival`;
}));
add('script-function-parameter-binding', ['SCRIPT-01'], () => new Function('location', 'return location')('parameter-value'));
add('script-async-function', ['SCRIPT-01'], () => Object.getPrototypeOf(async function() {}).constructor('return location.origin')());
add('script-generator-function', ['SCRIPT-01'], () => Object.getPrototypeOf(function*() {}).constructor('yield location.origin')().next().value);
add('script-data-url-content', ['SCRIPT-01'], () => useFrame('data:text/html,<p>data-page</p>', frame => ({ loaded: Boolean(frame.contentWindow) })));
add('script-javascript-url', ['NAV-03', 'SCRIPT-01'], () => arrival(frame => {
  const link = frame.contentDocument.createElement('a');
  link.href = `javascript:location.href='${AUTH}/lab/arrival'`;
  frame.contentDocument.body.append(link);
  link.click();
}));
add('cookie-path-and-expiry', ['COOKIE-01', 'COOKIE-05'], async () => {
  document.cookie = 'pathCookie=root; Path=/; SameSite=Lax';
  document.cookie = 'pathCookie=lab; Path=/lab; SameSite=Lax';
  document.cookie = 'expired=gone; Max-Age=0; Path=/; SameSite=Lax';
  const result = await (await fetch('/lab/cookie')).json();
  return { order: result.cookie.match(/pathCookie=[^;]+/g), expired: result.cookie.includes('expired=') };
});
add('cookie-store-api', ['COOKIE-02', 'COOKIE-05'], async () => {
  if (!globalThis.cookieStore) return { supported: false };
  await cookieStore.set({ name: 'storeCookie', value: 'value', path: '/', sameSite: 'lax' });
  const value = await cookieStore.get('storeCookie');
  const sent = (await (await fetch('/lab/cookie')).json()).cookie.includes('storeCookie=value');
  await cookieStore.delete('storeCookie');
  return { supported: true, value: value?.value, sent, deleted: await cookieStore.get('storeCookie') === null };
});
add('cookie-prefix-restrictions', ['COOKIE-04'], () => {
  document.cookie = '__Host-invalid=bad; Path=/';
  document.cookie = '__Secure-invalid=bad; Path=/';
  document.cookie = '__Host-valid=yes; Path=/; Secure';
  return { invalidHost: document.cookie.includes('__Host-invalid='), invalidSecure: document.cookie.includes('__Secure-invalid='), valid: document.cookie.includes('__Host-valid=yes') };
});
add('cookie-httpOnly-cache-and-idb-inspection', ['COOKIE-03', 'ORIGIN-01'], async () => {
  await fetch(`/lab/cookie?set=${encodeURIComponent('hidden=fixture-secret; HttpOnly; Path=/')}`);
  const cacheNames = await caches.keys();
  const databaseNames = (await indexedDB.databases()).map(database => database.name);
  const visible = document.cookie;
  return { leaked: visible.includes('fixture-secret'), unexpectedCache: cacheNames.filter(name => !name.startsWith('lab-')), databases: databaseNames };
});
add('origin-fetch-metadata', ['ORIGIN-02', 'COOKIE-04'], async () => {
  const response = await fetch(`${OTHER}/lab/cors?allow=*`);
  return await response.json();
});
add('origin-forged-origin-header', ['ORIGIN-02'], async () => {
  const response = await fetch('/lab/echo', { method: 'POST', headers: { origin: AUTH, 'x-spike-hop': 'forged' }, body: 'request' });
  const result = await response.json();
  return { origin: result.headers.origin, host: result.headers.host };
});
add('origin-storage-cross-frame', ['ORIGIN-01'], () => useFrame(`${OTHER}/lab`, frame => {
  try { return { denied: false, value: frame.contentWindow.localStorage.getItem('private') }; }
  catch (error) { return { denied: true, error: error.name }; }
}));
add('origin-proxy-management-access', ['ORIGIN-02', 'ESCAPE-02'], async () => {
  try {
    const response = await fetch('https://proxy.spike.test:19443/control/events');
    await response.json();
    return { denied: false };
  } catch (error) { return { denied: true }; }
});
add('storage-broadcast-channel', ['ORIGIN-01'], async () => {
  const sender = new BroadcastChannel('lab-channel');
  const receiver = new BroadcastChannel('lab-channel');
  try { return await deadline(new Promise(resolve => { receiver.onmessage = event => resolve(event.data); sender.postMessage('roundtrip'); })); }
  finally { sender.close(); receiver.close(); }
});
add('storage-web-locks', ['ORIGIN-01'], async () => navigator.locks.request('lab-lock', () => ({ held: true })));
add('storage-opfs', ['ORIGIN-01', 'CLEAN-01'], async () => {
  const root = await navigator.storage.getDirectory();
  const handle = await root.getFileHandle('lab.txt', { create: true });
  const writer = await handle.createWritable();
  await writer.write('opfs-value');
  await writer.close();
  try { return await (await handle.getFile()).text(); }
  finally { await root.removeEntry('lab.txt'); }
});
add('message-transfer-arraybuffer', ['ORIGIN-01', 'SW-10'], async () => {
  const channel = new MessageChannel();
  const bytes = new Uint8Array([1, 2, 3]);
  try {
    const reply = new Promise(resolve => { channel.port2.onmessage = event => resolve([...new Uint8Array(event.data)]); });
    channel.port1.postMessage(bytes.buffer, [bytes.buffer]);
    return { bytes: await deadline(reply), detached: bytes.byteLength === 0 };
  } finally { channel.port1.close(); channel.port2.close(); }
});
add('sw-unchanged-update', ['SW-05', 'SW-09'], async () => {
  const registration = await installSW();
  const active = registration.active;
  await registration.update();
  return { sameWorker: active === registration.active, waiting: Boolean(registration.waiting), version: (await swMessage('version')).version };
});
add('sw-module-type', ['SW-04', 'SW-08'], async () => {
  const registration = await navigator.serviceWorker.register('/lab/sw.js?module=1', { type: 'module', scope: '/' });
  const worker = registration.installing ?? registration.active;
  if (worker?.state !== 'activated') await deadline(new Promise((resolve, reject) => {
    const changed = () => {
      if (worker.state === 'activated') { worker.removeEventListener('statechange', changed); resolve(); }
      if (worker.state === 'redundant') { worker.removeEventListener('statechange', changed); reject(new Error('Module SW failed')); }
    };
    worker.addEventListener('statechange', changed);
    changed();
  }));
  return { active: Boolean(registration.active) };
});
add('sw-navigation-preload', ['SW-08'], async () => {
  const registration = await installSW();
  await registration.navigationPreload.enable();
  await registration.navigationPreload.setHeaderValue('fixture-preload');
  return await registration.navigationPreload.getState();
});
add('sw-enumeration', ['SW-04', 'SW-12'], async () => {
  await installSW();
  return (await navigator.serviceWorker.getRegistrations()).map(registration => ({ scope: registration.scope, scriptURL: registration.active?.scriptURL }));
});
add('sw-cache-query-and-vary', ['SW-11', 'HTTP-03'], async () => {
  const cache = await caches.open('lab-cache');
  await cache.put(`${APP}/lab/value?x=1`, new Response('query-value'));
  return { exact: Boolean(await cache.match(`${APP}/lab/value?x=2`)), ignoreSearch: await (await cache.match(`${APP}/lab/value?x=2`, { ignoreSearch: true })).text(), keys: (await cache.keys()).map(request => request.url) };
});
add('policy-canvas-taint', ['POLICY-01', 'ORIGIN-01'], async () => {
  const image = new Image();
  image.src = `${OTHER}/lab/pixel.svg`;
  await image.decode();
  const canvas = document.createElement('canvas');
  canvas.getContext('2d').drawImage(image, 0, 0);
  try { canvas.toDataURL(); return { blocked: false }; }
  catch (error) { return { blocked: true, error: error.name }; }
}, { policyChange: 'readable-without-third-party-cookies', directExpected: { blocked: true, error: 'SecurityError' }, proxyExpected: { blocked: false } });
add('policy-sab-availability', ['POLICY-01'], () => ({ isolated: crossOriginIsolated, sharedArrayBuffer: typeof SharedArrayBuffer }));
add('wasm-minimal-module', ['SCRIPT-01'], async () => {
  const bytes = new Uint8Array([0,97,115,109,1,0,0,0]);
  const module = await WebAssembly.compile(bytes);
  const instance = await WebAssembly.instantiate(module);
  return { exports: Object.keys(instance.exports) };
});
add('websocket-close-semantics', ['WS-02'], () => deadline(new Promise((resolve, reject) => {
  const socket = new WebSocket('ws://localhost:19401/echo-ws', 'fixture-echo');
  socket.onopen = () => socket.close(1000, 'fixture-close');
  socket.onclose = event => resolve({ code: event.code, reason: event.reason, clean: event.wasClean, state: socket.readyState });
  socket.onerror = () => reject(new Error('WebSocket failed'));
})));
add('websocket-invalid-protocol', ['WS-02'], () => {
  try { new WebSocket('ws://localhost:19401/echo-ws', ['duplicate', 'duplicate']); return false; }
  catch (error) { return error.name; }
});
add('websocket-handshake-failure', ['WS-02'], () => deadline(new Promise(resolve => {
  const socket = new WebSocket('ws://localhost:19401/lab/reject-ws');
  let opened = false;
  let error = false;
  socket.onopen = () => { opened = true; socket.close(); };
  socket.onerror = () => { error = true; };
  socket.onclose = event => resolve({ opened, error, code: event.code });
})));
add('http-large-download-integrity', ['HTTP-02', 'PERF-01'], async () => {
  const bytes = await (await fetch('/lab/bytes')).arrayBuffer();
  const hash = await crypto.subtle.digest('SHA-256', bytes);
  return { length: bytes.byteLength, digest: [...new Uint8Array(hash)] };
});
add('fetch-referrer-and-authorization', ['HTTP-01', 'ORIGIN-02'], async () => {
  const result = await (await fetch('/lab/echo', { headers: { authorization: 'Bearer fixture-only' }, referrerPolicy: 'no-referrer' })).json();
  return { referrer: result.headers.referer ?? null, authorization: result.headers.authorization };
});
