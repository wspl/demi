const APP = 'http://localhost:19401';
const AUTH = 'https://auth.upstream.test:19444';
const OTHER = 'https://other.upstream.test:19444';
// A site on the internet, for the preview's local network checks (it runs on loopback too).
const PUBLIC = 'https://app.upstream.test:19444';
const catalog = [];
const add = (id, groups, run, expectations = {}) => catalog.push({ id, groups, run, ...expectations });
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));

async function deadline(work, ms = 5000) {
  let timer;
  try {
    return await Promise.race([work, new Promise((_, reject) => { timer = setTimeout(() => reject(new Error('Fixture operation timed out')), ms); })]);
  } finally { clearTimeout(timer); }
}
async function useFrame(url, action) {
  const frame = document.createElement('iframe');
  try {
    await deadline(new Promise((resolve, reject) => {
      frame.onload = event => {
        try { if (url !== 'about:blank' && frame.contentWindow.location.href === 'about:blank') return; }
        catch (error) { if (error.name !== 'SecurityError') throw error; }
        resolve(event);
      };
      frame.onerror = () => reject(new Error('Frame load failed'));
      frame.src = url;
      document.body.append(frame);
    }));
    return await action(frame);
  } finally { frame.remove(); }
}
async function useWorker(url, options, message) {
  const worker = new Worker(url, options);
  try {
    return await deadline(new Promise((resolve, reject) => {
      worker.onmessage = event => resolve(event.data);
      worker.onerror = event => reject(new Error(event.message || 'Worker failed'));
      worker.postMessage(message);
    }));
  } finally { worker.terminate(); }
}
async function swMessage(message, worker = navigator.serviceWorker.controller) {
  const channel = new MessageChannel();
  try {
    return await deadline(new Promise((resolve, reject) => {
      channel.port1.onmessage = event => resolve(event.data);
      channel.port1.onmessageerror = reject;
      worker.postMessage(message, [channel.port2]);
    }));
  } finally { channel.port1.close(); channel.port2.close(); }
}
async function installSW(query = '', scope = '/') {
  const registration = await navigator.serviceWorker.register(`/lab/sw.js${query}`, { scope });
  await navigator.serviceWorker.ready;
  if (navigator.serviceWorker.controller?.scriptURL !== `${APP}/lab/sw.js${query}`) {
    const worker = registration.installing ?? registration.waiting ?? registration.active;
    if (worker.state !== 'activated') {
      await deadline(new Promise((resolve, reject) => {
        const changed = () => {
          if (worker.state === 'activated' || worker.state === 'redundant') {
            worker.removeEventListener('statechange', changed);
            if (worker.state === 'activated') resolve();
            else reject(new Error('SW became redundant'));
          }
        };
        worker.addEventListener('statechange', changed);
      }));
    }
    await deadline((async () => {
      while (navigator.serviceWorker.controller?.scriptURL !== `${APP}/lab/sw.js${query}`) await sleep(20);
    })());
  }
  return registration;
}
async function arrival(action) {
  return useFrame('about:blank', async frame => {
    let listener;
    try {
      return await deadline(new Promise((resolve, reject) => {
        listener = event => {
          if (event.source === frame.contentWindow && event.data?.labArrival) resolve({ method: event.data.method, url: event.data.url });
        };
        window.addEventListener('message', listener);
        try { action(frame); } catch (error) { reject(error); }
      }));
    } finally { window.removeEventListener('message', listener); }
  });
}

for (const method of ['GET', 'HEAD', 'POST', 'PUT', 'PATCH', 'DELETE', 'OPTIONS']) {
  add(`http-method-${method.toLowerCase()}`, ['HTTP-01'], async () => {
    const response = await fetch('/lab/echo', { method, ...(['GET', 'HEAD'].includes(method) ? {} : { body: '中文=unchanged', headers: { 'content-type': 'text/plain' } }) });
    if (method === 'HEAD') return { status: response.status, text: await response.text() };
    const result = await response.json();
    return { method: result.method, body: result.body, bytes: result.bytes, origin: result.headers.origin ?? null, host: result.headers.host };
  });
}
for (const code of [200, 204, 205, 304, 400, 401, 404, 500]) {
  add(`http-status-${code}`, ['HTTP-01'], async () => {
    const response = await fetch(`/lab/status?code=${code}`);
    return { status: response.status, statusHeader: response.headers.get('x-test-status'), text: await response.text() };
  });
}
add('http-binary-upload', ['HTTP-01', 'HTTP-02'], async () => {
  const body = Uint8Array.from({ length: 65536 }, (_, index) => index % 251);
  const response = await (await fetch('/lab/echo', { method: 'POST', body })).json();
  return { bytes: response.bytes, sha256: response.sha256 };
});
add('http-multipart', ['HTTP-01', 'SW-02'], async () => {
  const body = new FormData();
  body.set('message', '中文');
  body.set('file', new File(['file bytes'], 'upload.txt', { type: 'text/plain' }));
  const result = await (await fetch('/lab/echo', { method: 'POST', body })).json();
  return { method: result.method, type: result.headers['content-type'].split(';')[0], message: result.body.includes('中文'), file: result.body.includes('file bytes'), name: result.body.includes('filename="upload.txt"') };
});
add('http-request-clone', ['HTTP-01'], async () => {
  const request = new Request(`${APP}/lab/echo`, { method: 'POST', body: 'clone-body' });
  const copy = request.clone();
  const response = await fetch(request);
  return { echoed: (await response.json()).body, copy: await copy.text(), consumed: request.bodyUsed };
});
add('http-request-consumed-rejects', ['HTTP-01'], async () => {
  const request = new Request(`${APP}/lab/echo`, { method: 'POST', body: 'consumed' });
  await request.text();
  try { await fetch(request); return false; } catch (error) { return error.name === 'TypeError'; }
});
add('http-range-and-download', ['HTTP-02'], async () => {
  const response = await fetch('/lab/bytes', { headers: { range: 'bytes=123-456' } });
  return { status: response.status, range: response.headers.get('content-range'), bytes: [...new Uint8Array(await response.arrayBuffer())] };
});
for (const encoding of ['gzip', 'br']) {
  add(`http-compressed-${encoding}`, ['HTTP-02'], async () => {
    const response = await fetch(`/lab/compressed?encoding=${encoding}`);
    const body = new DOMParser().parseFromString(await response.text(), 'text/html');
    return body.querySelector('#compressed')?.textContent ?? 'missing';
  });
}
add('http-etag-revalidation', ['HTTP-03'], async () => {
  const first = await fetch('/lab/cache', { cache: 'no-store' });
  const etag = first.headers.get('etag');
  const second = await fetch('/lab/cache', { headers: { 'if-none-match': etag }, cache: 'no-store' });
  return { etag, status: second.status };
});
add('http-cache-vary', ['HTTP-03'], async () => {
  const values = [];
  for (const flavor of ['one', 'two', 'one']) values.push((await (await fetch('/lab/cache', { headers: { 'x-flavor': flavor } })).json()).flavor);
  return values;
});
for (const code of [301, 302, 303, 307, 308]) {
  add(`redirect-post-${code}`, ['NAV-01', 'HTTP-01'], async () => {
    const response = await fetch(`/lab/redirect?code=${code}&to=${encodeURIComponent(`${AUTH}/lab/redirect?code=307&to=${encodeURIComponent(`${APP}/lab/echo`)}`)}`, { method: 'POST', body: 'redirect-body' });
    const data = await response.json();
    return { method: data.method, body: data.body, url: response.url, redirected: response.redirected };
  });
}
add('redirect-manual', ['NAV-01'], async () => {
  const response = await fetch('/lab/redirect', { redirect: 'manual' });
  return { type: response.type, status: response.status, location: response.headers.get('location'), text: await response.text() };
});
add('redirect-error', ['NAV-01'], async () => {
  try { await fetch('/lab/redirect', { redirect: 'error' }); return false; } catch (error) { return error.name === 'TypeError'; }
});
// A cross-origin no-cors response reaches a preview readable, so that everything can come over
// the direct connection; across sites it carries no cookie (docs/preview-domain.md
// § 转发器与预览中转), and it shows the headers a CORS request without credentials would.
add('cors-no-cors-cross-site-without-cookie', ['ORIGIN-01', 'POLICY-01'], async () => {
  const setCookie = value => fetch(`${OTHER}/lab/cookie?set=${encodeURIComponent(`noCorsProbe=${value}; Path=/; SameSite=None; Secure`)}`, { mode: 'no-cors', credentials: 'include' });
  await setCookie('1');
  try {
    const response = await fetch(`${OTHER}/lab/cors`, { mode: 'no-cors' });
    return { type: response.type, status: response.status, text: await response.text(), public: response.headers.get('x-public'), secret: response.headers.get('x-secret') };
  } finally { await setCookie('; Max-Age=0'); }
}, {
  policyChange: 'readable-without-third-party-cookies',
  directExpected: { type: 'opaque', status: 0, text: '', public: null, secret: null },
  proxyExpected: { type: 'basic', status: 200, text: '{"preflight":false,"cookie":"","origin":""}', public: 'visible', secret: null },
});
// A page on the internet reaches neither the Host's own services nor a name that resolves to
// them; its own site it does.
add('local-network-from-public-page', ['POLICY-01'], async () => {
  const frame = document.createElement('iframe');
  let listener;
  try {
    return await deadline(new Promise(resolve => {
      listener = event => { if (event.data?.localNetwork) resolve(event.data.localNetwork); };
      window.addEventListener('message', listener);
      frame.src = `${PUBLIC}/lab/local-network-probe`;
      document.body.append(frame);
    }), 10000);
  } finally { window.removeEventListener('message', listener); frame.remove(); }
}, { policyChange: 'local-network', directExpected: ['loaded', 'loaded', 'loaded'], proxyExpected: ['TypeError', 'TypeError', 'loaded'] });
add('http-reused-connection-closed', ['HTTP-02'], async () => {
  const statuses = [];
  for (let attempt = 0; attempt < 2; attempt++) statuses.push((await fetch('/lab/closing-connection', { cache: 'no-store' })).status);
  return statuses;
});
add('cors-preflight-and-exposure', ['ORIGIN-01', 'POLICY-01'], async () => {
  const response = await fetch(`${OTHER}/lab/cors?allow=${encodeURIComponent(APP)}`, { method: 'PUT', headers: { 'x-probe': 'value' }, body: 'cors' });
  return { status: response.status, public: response.headers.get('x-public'), secret: response.headers.get('x-secret'), origin: (await response.json()).origin };
});
add('cors-wildcard-credentials-rejected', ['ORIGIN-01', 'POLICY-01'], async () => {
  try { await fetch(`${OTHER}/lab/cors?allow=*&credentials=1`, { credentials: 'include' }); return false; } catch (error) { return error.name === 'TypeError'; }
});
add('url-relative-query-encoding', ['URL-01', 'URL-02'], async () => {
  const response = await fetch('./lab/echo?x=a%2Fb&x=%E4%B8%AD&empty=#fragment');
  return { url: response.url, path: (await response.json()).path };
});
add('url-document-views', ['URL-01', 'SCRIPT-01'], () => ({ location: location.href, origin: location.origin, documentURL: document.URL, baseURI: document.baseURI }));
// Pages parse URLs with an <a>: set href, read the parts (Microsoft Learn builds its
// breadcrumb address this way).
add('url-anchor-components', ['URL-01'], () => {
  const link = document.createElement('a');
  link.href = '/lab/parsed?q=1#part';
  const parts = { origin: link.origin, protocol: link.protocol, host: link.host, hostname: link.hostname, port: link.port, pathname: link.pathname, search: link.search, hash: link.hash };
  link.pathname = '/lab/changed';
  return { ...parts, changed: link.href, empty: document.createElement('a').host };
});
// The browser fetches Media Session artwork itself (YouTube sets it for the playing video).
add('url-media-session-artwork', ['URL-01'], async () => {
  const metadata = new MediaMetadata({ title: 'Probe', artwork: [{ src: '/public-asset.txt', sizes: '96x96', type: 'image/png' }] });
  metadata.artwork = [...metadata.artwork, { src: 'cover.png' }];
  return metadata.artwork.map(image => ({ src: image.src, sizes: image.sizes }));
});
// Selectors on URL attributes match what the page wrote (Bilibili finds its frames this way).
add('url-attribute-selectors', ['URL-01'], () => {
  const link = document.createElement('a');
  link.href = '/lab/selected';
  link.id = 'selected-link';
  const external = document.createElement('a');
  external.href = 'https://example.com/report.pdf';
  const style = document.createElement('style');
  style.textContent = 'a[href="/lab/selected"] { outline-color: rgb(1, 2, 3) }';
  document.body.append(link, external);
  document.head.append(style);
  try {
    return {
      exact: document.querySelector('a[href="/lab/selected"]')?.id ?? null,
      scheme: [...document.querySelectorAll('a[href^="http"]')].filter(element => element === link || element === external).map(element => element === link ? 'link' : 'external'),
      suffix: external.matches('[href$=".pdf"]'),
      closest: link.closest('[href^="/lab/"]') === link,
      styled: getComputedStyle(link).outlineColor,
      selectorText: style.sheet.cssRules[0].selectorText,
    };
  } finally {
    link.remove();
    external.remove();
    style.remove();
  }
});
// A frame without an address: a clean window and a document of the page's origin
// (Cloudflare's challenge lists such a window's properties, reCAPTCHA takes its JSON).
add('origin-local-frame-window', ['ORIGIN-01'], async () => {
  const frame = document.createElement('iframe');
  frame.style.display = 'none';
  document.body.append(frame);
  try {
    const view = frame.contentWindow;
    const frameDocument = frame.contentDocument;
    view.marker = 1;
    frameDocument.open();
    frameDocument.write('<p id="written">written</p>');
    frameDocument.close();
    const message = await new Promise(resolve => {
      view.addEventListener('message', event => resolve(event.data), { once: true });
      view.postMessage('hello', '*');
    });
    return {
      json: view.JSON.stringify({ a: 1 }),
      call: typeof view.Function.prototype.call,
      self: view.window === view && view.self === view,
      parent: view.parent === window,
      frameElement: view.frameElement === frame,
      document: frameDocument === view.document && frameDocument.defaultView === view,
      written: frameDocument.getElementById('written')?.textContent,
      marker: view.marker,
      leaked: window.marker,
      href: view.location.href,
      pageGlobal: 'labReady' in view,
      listed: Object.getOwnPropertyNames(view).includes('JSON'),
      timer: await new Promise(resolve => view.setTimeout(() => resolve('ran'), 0)),
      message,
    };
  } finally {
    frame.remove();
  }
});
// A destructuring read of the window (Transcend's consent manager checks its host this way).
add('origin-destructured-location', ['ORIGIN-01'], () => {
  const { location: place, top: upper } = window;
  return { origin: place.origin, href: place.href, topIsSelf: upper === window };
});
// Markup parsed outside the page and moved in (DOMPurify parses with DOMParser).
add('url-parsed-markup-addresses', ['URL-01'], async () => {
  const parsed = new DOMParser().parseFromString('<img id="parsed" src="/public-asset.txt?parsed">', 'text/html').getElementById('parsed');
  const fragment = document.createRange().createContextualFragment('<img id="contextual" src="/public-asset.txt?contextual">');
  const images = [document.importNode(parsed, true), fragment.firstElementChild];
  document.body.append(...images);
  try {
    return images.map(image => ({ attribute: image.getAttribute('src'), property: image.src }));
  } finally {
    for (const image of images) image.remove();
  }
});
add('url-xhr-response', ['URL-01', 'HTTP-01'], () => deadline(new Promise((resolve, reject) => {
  const request = new XMLHttpRequest();
  request.open('GET', `${APP}/lab/echo`);
  request.onload = () => resolve({ url: request.responseURL, status: request.status });
  request.onerror = () => reject(new Error('XHR failed'));
  request.send();
})));
add('script-dynamic-import-meta', ['SCRIPT-02'], async () => {
  const module = await import(`${APP}/lab/value.js`);
  return { value: module.value, url: module.url };
});
add('script-direct-eval-lexical', ['SCRIPT-01'], () => {
  const lexical = 42;
  return eval('({lexical, origin: location.origin})');
});
add('script-function-constructor', ['SCRIPT-01'], () => new Function('return location.origin')());
for (const [id, action] of [
  ['new-realm-function', frame => frame.contentWindow.Function(`location.href='${AUTH}/lab/arrival'`)()],
  ['new-realm-eval', frame => frame.contentWindow.eval(`location.href='${AUTH}/lab/arrival'`)],
  ['new-realm-location', frame => { frame.contentWindow.location.href = `${AUTH}/lab/arrival`; }],
  ['frames-index-location', () => { window.frames[0].location.href = `${AUTH}/lab/arrival`; }],
  ['document-write-link', frame => { frame.contentDocument.write(`<a id="go" href="${AUTH}/lab/arrival">go</a>`); frame.contentDocument.close(); frame.contentDocument.querySelector('#go').click(); }],
  ['srcdoc-inline-navigation', frame => { frame.srcdoc = `<script>location.href='${AUTH}/lab/arrival'<\/script>`; }],
  ['dynamic-meta-refresh', frame => { frame.contentDocument.head.innerHTML = `<meta http-equiv="refresh" content="0;url=${AUTH}/lab/arrival">`; }],
  ['form-submit', frame => { frame.contentDocument.body.innerHTML = `<form action="${AUTH}/lab/arrival" method="post"><input name="field" value="body"></form>`; frame.contentDocument.querySelector('form').submit(); }],
]) add(`navigation-${id}`, ['NAV-02', 'NAV-03', 'SCRIPT-01', 'ESCAPE-02'], () => arrival(action));
for (const kind of ['header', 'meta']) add(`navigation-refresh-${kind}`, ['NAV-03'], () => arrival(frame => { frame.src = `/lab/refresh?kind=${kind}`; }));
add('history-push-replace-hash', ['NAV-04'], () => {
  history.pushState({ value: 1 }, '', '/lab?first=1#one');
  const first = { state: history.state, url: location.href };
  history.replaceState({ value: 2 }, '', '/lab?second=2#two');
  return { first, second: { state: history.state, url: location.href } };
});
for (const [id, url, options, message] of [
  ['classic', '/lab/worker.js', {}, 'fetch'],
  ['module', '/lab/worker-module.js', { type: 'module' }, 'fetch'],
  ['nested', '/lab/worker.js', {}, 'nested'],
  ['websocket', '/lab/worker.js', {}, 'ws'],
]) add(`worker-${id}`, ['WORKER-01', 'WS-01'], () => useWorker(url, options, message));
add('worker-blob', ['WORKER-01', 'ESCAPE-01'], async () => {
  const blob = new Blob([`self.onmessage=async()=>postMessage({url:(await fetch('${APP}/lab/echo')).url});`], { type: 'text/javascript' });
  const url = URL.createObjectURL(blob);
  try { return await useWorker(url, {}, 'fetch'); }
  finally { URL.revokeObjectURL(url); }
});
add('stream-incremental-cancel', ['STREAM-01', 'HTTP-02', 'CLEAN-01'], async () => {
  const response = await fetch('/lab/stream');
  const reader = response.body.getReader();
  try {
    const first = new TextDecoder().decode((await reader.read()).value);
    const second = new TextDecoder().decode((await reader.read()).value);
    return { first, second };
  } finally { await reader.cancel(); reader.releaseLock(); }
});
add('fetch-abort-reason', ['HTTP-01', 'RECOVER-01'], async () => {
  const controller = new AbortController();
  controller.abort('fixture-abort');
  try { await fetch('/lab/echo', { signal: controller.signal }); return 'not aborted'; }
  catch (error) { return error; }
});
add('cookie-httpOnly-and-delete', ['COOKIE-02', 'COOKIE-03', 'COOKIE-05'], async () => {
  await fetch(`/lab/cookie?set=${encodeURIComponent('private=secret; HttpOnly; Path=/; SameSite=Lax')}`);
  document.cookie = 'visible=one; Path=/; SameSite=Lax';
  const read = document.cookie;
  const first = (await (await fetch('/lab/cookie')).json()).cookie;
  document.cookie = 'visible=gone; Max-Age=0; Path=/; SameSite=Lax';
  const second = (await (await fetch('/lab/cookie')).json()).cookie;
  return { exposed: read.includes('private='), sent: first.includes('private=secret'), immediate: first.includes('visible=one'), deleted: !second.includes('visible=') };
});
add('cookie-credentials-omit', ['COOKIE-01', 'COOKIE-04'], async () => {
  document.cookie = 'credential=present; Path=/; SameSite=Lax';
  return (await (await fetch('/lab/cookie', { credentials: 'omit' })).json()).cookie;
});
add('cookie-parent-domain', ['COOKIE-01', 'COOKIE-04'], async () => useFrame(`${AUTH}/lab`, async frame => {
  return new Promise(resolve => {
    const listener = event => {
      if (event.source !== frame.contentWindow || !event.data?.cookieResult) return;
      window.removeEventListener('message', listener);
      resolve(event.data.cookieResult);
    };
    window.addEventListener('message', listener);
    frame.contentWindow.postMessage({ labCommand: 'parent-cookie' }, '*');
  });
}));
add('storage-local-idb-cache', ['ORIGIN-01', 'CLEAN-01'], async () => {
  localStorage.setItem('lab', 'local');
  sessionStorage.setItem('lab', 'session');
  const cache = await caches.open('lab-values');
  await cache.put('/lab/stored', new Response('cache-value'));
  const db = await new Promise((resolve, reject) => {
    const request = indexedDB.open('lab-values', 1);
    request.onupgradeneeded = () => request.result.createObjectStore('values');
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
  try {
    await new Promise((resolve, reject) => {
      const transaction = db.transaction('values', 'readwrite');
      transaction.objectStore('values').put('idb-value', 'key');
      transaction.oncomplete = resolve;
      transaction.onerror = () => reject(transaction.error);
    });
    const idb = await new Promise((resolve, reject) => {
      const request = db.transaction('values').objectStore('values').get('key');
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error);
    });
    return { local: localStorage.getItem('lab'), session: sessionStorage.getItem('lab'), cached: await (await cache.match('/lab/stored')).text(), idb };
  } finally { db.close(); }
});
add('message-exact-target-origin', ['ORIGIN-01', 'AUTH-01'], async () => useFrame(`${AUTH}/lab`, async frame => {
  let listener;
  try {
    return await deadline(new Promise(resolve => {
      listener = event => {
        if (event.source === frame.contentWindow && event.data?.labReply) resolve({ value: event.data.labReply, origin: event.origin });
      };
      window.addEventListener('message', listener);
      frame.contentWindow.postMessage({ labCommand: 'echo' }, AUTH);
    }));
  } finally { window.removeEventListener('message', listener); }
}));
for (const kind of ['csp', 'sri', 'tt']) {
  add(`policy-${kind}-allowed-behavior`, ['POLICY-01'], async () => useFrame(`/lab/policy?kind=${kind}`, frame => {
    const target = frame.contentWindow;
    return { allowed: target.policyAllowed ?? null, forbidden: target.policyForbidden ?? null, integrity: target.integrityWorked ?? null, trustedTypesBlocked: target.ttBlocked ?? null };
  }), kind === 'csp' ? { policyChange: 'csp-removed', proxyExpected: { allowed: true, forbidden: true, integrity: null, trustedTypesBlocked: null } } : kind === 'tt' ? { policyChange: 'csp-removed', proxyExpected: { allowed: null, forbidden: null, integrity: null, trustedTypesBlocked: false } } : {});
}
add('policy-sri-invalid-blocked', ['POLICY-01'], async () => useFrame('/lab/policy?kind=sri&bad=1', frame => frame.contentWindow.integrityWorked === undefined));
add('css-url-resource', ['SCRIPT-02', 'ESCAPE-01'], async () => {
  const element = document.createElement('div');
  element.className = 'probe-image';
  element.style.cssText = 'width:10px;height:10px';
  const link = document.createElement('link');
  link.rel = 'stylesheet';
  link.href = '/lab/style.css';
  try {
    await deadline(new Promise((resolve, reject) => { link.onload = resolve; link.onerror = reject; document.head.append(link); }));
    document.body.append(element);
    await sleep(100);
    return getComputedStyle(element).backgroundImage;
  } finally { element.remove(); link.remove(); }
});
add('sw-register-metadata', ['SW-04', 'SW-06'], async () => {
  const registration = await installSW();
  return { scope: registration.scope, url: registration.active.scriptURL, state: registration.active.state, reply: await swMessage('version') };
});
add('sw-request-and-stream', ['SW-06', 'SW-07'], async () => {
  await installSW();
  return { request: await (await fetch('/lab/sw-info')).json(), stream: await (await fetch('/lab/sw-stream')).text() };
});
add('sw-network-and-cache-addall', ['SW-08'], async () => {
  await installSW();
  return { response: await (await fetch('/lab/sw-external')).json(), cached: await (await fetch('/lab/sw-asset')).text() };
});
add('sw-cache-add-absolute', ['SW-08', 'ESCAPE-01'], async () => { await installSW(); return swMessage('cache-absolute'); });
add('sw-clients-logical-urls', ['SW-10'], async () => { await installSW(); return (await swMessage('clients')).map(client => ({ url: client.url, type: client.type })); });
add('sw-response-rejection', ['SW-07', 'RECOVER-01'], async () => {
  await installSW();
  try { await fetch('/lab/sw-throw'); return false; } catch (error) { return error.name === 'TypeError'; }
});
add('sw-invalid-mime', ['SW-09'], async () => {
  try { await navigator.serviceWorker.register('/lab/sw-invalid.js'); return false; }
  catch (error) { return error.name; }
});
add('sw-install-failure', ['SW-09'], async () => {
  const existing = await installSW();
  try {
    const failed = await navigator.serviceWorker.register('/lab/sw-failed.js', { scope: '/' });
    if (failed.installing) await deadline(new Promise(resolve => { const worker = failed.installing; const check = () => { if (worker.state === 'redundant') { worker.removeEventListener('statechange', check); resolve(); } }; worker.addEventListener('statechange', check); check(); }));
  } catch { /* A rejected install is the intended fixture outcome. The old worker must remain usable. */ }
  return { oldActive: existing.active?.scriptURL, response: await (await fetch('/lab/sw-info')).json() };
});
add('sw-unregister-controller-lifetime', ['SW-04', 'SW-12'], async () => {
  const registration = await installSW();
  const unregistered = await registration.unregister();
  return { unregistered, stillControlled: Boolean(navigator.serviceWorker.controller), found: Boolean(await navigator.serviceWorker.getRegistration()) };
});
add('sw-native-background-api-surface', ['SW-13'], async () => {
  const registration = await installSW();
  return { sync: 'sync' in registration, periodicSync: 'periodicSync' in registration, push: 'pushManager' in registration, notificationPermission: Notification.permission };
});

window.addEventListener('message', event => {
  if (event.data?.labCommand === 'echo') event.source.postMessage({ labReply: 'message-roundtrip' }, event.origin);
  if (event.data?.labCommand === 'parent-cookie') {
    document.cookie = 'domainCookie=yes; Domain=upstream.test; Path=/; SameSite=None; Secure';
    event.source.postMessage({ cookieResult: { visible: document.cookie.includes('domainCookie=yes') } }, '*');
  }
});
Object.defineProperty(window, 'labCatalog', { get: () => catalog.map(({ run, ...specification }) => specification) });
window.runLabCase = async id => {
  const test = catalog.find(test => test.id === id);
  if (!test) throw new Error(`Unknown lab case: ${id}`);
  return deadline(Promise.resolve().then(test.run), 9000);
};
