add('native-location-descriptor-view', ['ESCAPE-02', 'NAV-02'], () => {
  const getter = Object.getOwnPropertyDescriptor(window, 'location').get;
  return { href: getter.call(window).href, configurable: Object.getOwnPropertyDescriptor(window, 'location').configurable };
});
add('native-location-descriptor-navigation', ['ESCAPE-02', 'NAV-02'], () => arrival(frame => {
  const nativeLocation = Object.getOwnPropertyDescriptor(frame.contentWindow, 'location').get.call(frame.contentWindow);
  nativeLocation.href = `${AUTH}/lab/arrival?native-descriptor=1`;
}));
add('unknown-origin-navigation', ['ESCAPE-01', 'URL-02'], () => arrival(frame => {
  frame.contentWindow.location = 'http://127.0.0.1:19401/lab/arrival?unknown-origin=1';
}));
add('worker-data-url', ['WORKER-01', 'ESCAPE-01'], () => useWorker(`data:text/javascript,${encodeURIComponent(`onmessage=async()=>postMessage((await(await fetch('${OTHER}/lab/cors?allow=*')).json()).origin)` )}`, {}, 'go'));
add('shared-worker-same-name-sharing', ['WORKER-01'], async () => {
  const url = URL.createObjectURL(new Blob(['let count=0;onconnect=e=>{const p=e.ports[0];p.onmessage=()=>p.postMessage(++count);p.start();}'], { type: 'text/javascript' }));
  const workers = [new SharedWorker(url, { name: 'sharing' }), new SharedWorker(url, { name: 'sharing' })];
  try {
    const result = [];
    for (const worker of workers) result.push(await deadline(new Promise(resolve => { worker.port.onmessage = e => resolve(e.data); worker.port.start(); worker.port.postMessage('go'); })));
    return result;
  } finally { for (const worker of workers) worker.port.close(); URL.revokeObjectURL(url); }
});
add('audio-worklet-network-module', ['WORKER-01', 'ESCAPE-01'], async () => {
  const context = new AudioContext();
  try { await context.audioWorklet.addModule(`${APP}/lab/audio-worklet.js`); return true; }
  finally { await context.close(); }
});
add('http-non-utf8-html', ['HTTP-02'], () => useFrame('/lab/latin1', frame => frame.contentDocument.querySelector('#latin').textContent));
add('http-cross-origin-authorization-redirect', ['HTTP-01', 'NAV-01'], async () => {
  const response = await fetch(`/lab/redirect?code=307&to=${encodeURIComponent(`${OTHER}/lab/echo`)}`, { headers: { authorization: 'Bearer fixture-only' } });
  return { authorization: (await response.json()).headers.authorization ?? null };
});
add('sw-narrow-scope-controller', ['SW-04'], async () => {
  const registration = await navigator.serviceWorker.register('/lab/sw.js', { scope: '/lab/narrow/' });
  const worker = registration.installing ?? registration.waiting ?? registration.active;
  if (worker.state !== 'activated') await deadline(new Promise(resolve => { const listener = () => { if (worker.state === 'activated') { worker.removeEventListener('statechange', listener); resolve(); } }; worker.addEventListener('statechange', listener); listener(); }));
  return useFrame('/lab/narrow/page', async frame => {
    const controller = frame.contentWindow.navigator.serviceWorker.controller;
    return { scope: registration.scope, controller: controller.scriptURL, message: await swMessage('version', controller) };
  });
});
add('sw-clients-navigate', ['SW-10'], async () => {
  await installSW();
  return useFrame('/lab/arrival?client-navigation=before', async frame => {
    const result = await swMessage({ navigate: `${APP}/lab/arrival?client-navigation=after` });
    await deadline((async () => { while (!frame.contentWindow.location.search.includes('after')) await sleep(20); })());
    return result;
  });
});
add('sw-clients-openwindow-without-activation', ['SW-10'], async () => { await installSW(); return swMessage({ openWindow: `${APP}/lab/arrival` }); });
add('sw-navigation-preload-actual-response', ['SW-08'], async () => {
  const registration = await installSW();
  await registration.navigationPreload.enable();
  return useFrame('/lab/preload-page', frame => ({ value: frame.contentDocument.querySelector('#preload').textContent, used: frame.contentDocument.querySelector('#preload-used')?.textContent ?? null }));
});
add('cookie-store-explicit-domain', ['COOKIE-01', 'COOKIE-02'], async () => {
  if (!globalThis.cookieStore) return 'unavailable';
  await cookieStore.set({ name: 'cookie-store-domain', value: 'yes', domain: 'localhost', path: '/' });
  const cookie = await cookieStore.get('cookie-store-domain');
  await cookieStore.delete({ name: 'cookie-store-domain', domain: 'localhost', path: '/' });
  return { value: cookie?.value ?? null, domain: cookie?.domain, deleted: (await cookieStore.get('cookie-store-domain')) === null };
});
add('cookie-partitioned-roundtrip', ['COOKIE-04'], async () => {
  await fetch(`/lab/cookie?set=${encodeURIComponent('partitioned=visible; Path=/; SameSite=None; Secure; Partitioned')}`);
  return { visible: document.cookie.includes('partitioned=visible'), sent: (await (await fetch('/lab/cookie')).json()).cookie.includes('partitioned=visible') };
});
add('dynamic-import-absolute-url', ['SCRIPT-02', 'URL-01'], async () => (await import(`${APP}/lab/value.js`)).value);
window.performanceProbe = async () => {
  const started = performance.now();
  const data = new Uint8Array(await (await fetch('/lab/large?bytes=8388608')).arrayBuffer());
  if (data.length !== 8388608 || data[17] !== 17) throw new Error('Large download mismatch');
  const downloadMs = performance.now() - started;
  const uploadStarted = performance.now();
  const uploaded = await (await fetch('/lab/upload', { method: 'POST', body: data })).json();
  const hash = [...new Uint8Array(await crypto.subtle.digest('SHA-256', data))].map(value => value.toString(16).padStart(2, '0')).join('');
  if (uploaded.sha256 !== hash) throw new Error('Large upload mismatch');
  const uploadMs = performance.now() - uploadStarted;
  const socket = new WebSocket('ws://localhost:19401/echo-ws');
  const wsMs = [];
  const concurrentStarted = performance.now();
  try {
    await deadline(new Promise((resolve, reject) => { socket.onopen = resolve; socket.onerror = reject; }));
    await Promise.all([
      ...Array.from({ length: 4 }, async () => { const body = await (await fetch('/lab/large?bytes=8388608')).arrayBuffer(); if (body.byteLength !== 8388608) throw new Error('Concurrent download mismatch'); }),
      (async () => { for (let index = 0; index < 20; index++) { const start = performance.now(); await deadline(new Promise(resolve => { socket.onmessage = () => resolve(); socket.send('latency'); })); wsMs.push(performance.now() - start); } })(),
    ]);
  } finally { socket.close(); }
  const memory = performance.memory ? { used: performance.memory.usedJSHeapSize, total: performance.memory.totalJSHeapSize } : null;
  return { downloadMs, uploadMs, concurrentMs: performance.now() - concurrentStarted, wsMs, bytes: data.length, hash, memory };
};

for (const kind of ['csp-origin', 'sri-cross']) add(`policy-explicit-${kind}`, ['POLICY-01'], () => useFrame(`/lab/policy?kind=${kind}`, frame => frame.contentWindow.integrityWorked ?? null));

add('same-host-second-port-navigation', ['URL-02', 'COOKIE-01'], () => arrival(frame => { frame.contentWindow.location = 'https://auth.upstream.test:19446/lab/arrival?second-port=1'; }));

add('native-location-reflection-semantics', ['ESCAPE-02', 'NAV-02'], () => {
  const result = {};
  let coercions = 0;
  const key = { toString() { coercions++; return 'location'; } };
  result.descriptor = Object.getOwnPropertyDescriptor(window, key).get.call(window).href;
  result.coercions = coercions;
  result.reflect = Reflect.get(window, key).href;
  result.reflectCoercions = coercions;
  for (const [name, action] of [
    ['getterReceiver', () => Reflect.get(window, 'location', {})],
    ['undefinedReceiver', () => Reflect.get(window, 'location', undefined)],
    ['locationReceiver', () => Reflect.get(location, 'href', {})],
    ['setterReceiver', () => Reflect.set(location, 'href', '#unexpected', {})],
    ['symbolURL', () => { location.href = Symbol('url'); }],
    ['invalidURL', () => { location.href = 'http://['; }],
  ]) {
    try { action(); result[name] = 'accepted'; }
    catch (error) { result[name] = error.name; }
  }
  const descriptor = Object.getOwnPropertyDescriptor(location, 'href');
  result.identity = descriptor.get === Reflect.getOwnPropertyDescriptor(location, 'href').get;
  result.lookup = location.__lookupGetter__('href').call(location);
  const symbol = Symbol('business');
  const ordinary = { [symbol]: 42, get href() { return this.value; } };
  result.symbol = Reflect.get(ordinary, symbol);
  result.business = Reflect.get(ordinary, 'href', { value: 7 });
  return result;
});
for (const reflection of ['reflect', 'descriptor', 'lookup']) {
  add(`native-location-coerced-${reflection}-navigation`, ['ESCAPE-02', 'NAV-02'], () => arrival(frame => {
    const key = { toString() { return 'href'; } };
    const target = frame.contentWindow.location;
    const url = `${AUTH}/lab/arrival?coerced=${reflection}`;
    if (reflection === 'reflect') Reflect.set(target, key, url);
    else if (reflection === 'descriptor') Object.getOwnPropertyDescriptor(target, key).set.call(target, url);
    else target.__lookupSetter__(key).call(target, url);
  }));
}
add('native-location-cross-origin-read', ['ESCAPE-02', 'NAV-02'], () => useFrame(`${OTHER}/lab/arrival`, frame => {
  const result = {};
  try { const target = frame.contentWindow.location; result.obtain = !!target; }
  catch (error) { result.obtain = error.name; }
  try { result.href = frame.contentWindow.location.href; }
  catch (error) { result.href = error.name; }
  return result;
}));

for (const mode of ['property', 'attribute', 'namespace', 'html']) {
  add(`resource-srcset-${mode}`, ['SCRIPT-01', 'ESCAPE-01'], async () => {
    const box = document.createElement('div');
    const source = ` /lab/pixel.svg?relative=1 1x, ${OTHER}/lab/pixel.svg?absolute=1 2x, data:image/svg+xml,%3Csvg%3E 0x, http://[ 3x `;
    if (mode === 'html') box.innerHTML = `<img srcset="${source}">`;
    else {
      const image = document.createElement('img');
      if (mode === 'property') image.srcset = source;
      if (mode === 'attribute') image.setAttribute('srcset', source);
      if (mode === 'namespace') image.setAttributeNS(null, 'srcset', source);
      box.append(image);
    }
    document.body.append(box);
    try {
      const image = box.querySelector('img');
      await image.decode();
      return { value: image.srcset, attribute: image.getAttribute('srcset'), namespace: image.getAttributeNS(null, 'srcset'), width: image.naturalWidth };
    } finally { box.remove(); }
  });
}

for (const type of ['classic', 'module']) {
  for (const encoding of ['percent', 'base64']) {
    add(`worker-data-${type}-${encoding}`, ['WORKER-01', 'ESCAPE-01'], async () => {
      const source = `onmessage=async()=>{let relative;try{await fetch('./invalid')}catch(e){relative=e.name}const response=await fetch('${OTHER}/lab/cors?allow=*');postMessage({text:'中文',origin:location.origin,href:location.href,name,selfOrigin:self.origin,relative,remote:(await response.json()).origin${type === 'module' ? ',module:import.meta.url' : ''}})}`;
      const url = encoding === 'percent' ? `data:text/javascript;charset=utf-8,${encodeURIComponent(source)}` : `data:text/javascript;charset=utf-8;base64,${btoa(String.fromCharCode(...new TextEncoder().encode(source)))}`;
      return useWorker(url, { type, name: 'data-fixture' }, 'go');
    });
  }
}
for (const kind of ['csp-hash', 'csp-hash-bad', 'csp-inline', 'csp-multiple', 'sri-cross-bad']) {
  add(`policy-adapted-${kind}`, ['POLICY-01'], () => useFrame(`/lab/policy?kind=${kind}`, frame => ({ allowed: frame.contentWindow.integrityWorked ?? null, forbidden: frame.contentWindow.policyForbidden ?? null })), kind.startsWith('csp-') ? { policyChange: 'csp-removed', proxyExpected: { allowed: APP, forbidden: true } } : {});
}
add('native-location-coerced-member-navigation', ['ESCAPE-02', 'NAV-02'], () => arrival(frame => {
  const property = { toString() { return 'location'; } };
  frame.contentWindow[property] = `${AUTH}/lab/arrival?coerced=member`;
}));
add('computed-property-coercion-order', ['SCRIPT-01'], () => {
  const log = [];
  const key = { toString() { log.push('key'); return 'value'; } };
  const object = { get value() { log.push('get'); return 7; } };
  const result = object[key];
  try { null[key]; } catch (error) { log.push(error.name); }
  return { result, log };
});

add('module-static-import-reexports-relative-dynamic', ['SCRIPT-02', 'ESCAPE-01'], async () => {
  const module = await import(`${APP}/lab/modules/chain.js`);
  const relative = await module.relative();
  return { value: module.value, named: module.named, importedURL: module.importedURL, relative: relative.url };
});
add('module-import-map-scopes-prefixes-url-keys', ['SCRIPT-02', 'ESCAPE-01'], () => useFrame('/lab/import-map-page', async frame => {
  await deadline((async () => { while (!frame.contentWindow.moduleMapResult) await sleep(20); })());
  return frame.contentWindow.moduleMapResult;
}));
add('worker-classic-importscripts-absolute', ['WORKER-01', 'ESCAPE-01'], () => useWorker(`${APP}/lab/worker-import.js`, {}, 'go'));

for (const method of ['property', 'setProperty', 'cssText', 'style-attribute', 'style-forward', 'style-text', 'sheet-replace', 'sheet-replaceSync']) {
  add(`css-resource-${method}`, ['SCRIPT-02', 'ESCAPE-01'], async () => {
    const element = document.createElement('div');
    element.id = 'css-resource';
    document.body.append(element);
    const style = document.createElement('style');
    const previousSheets = document.adoptedStyleSheets;
    const value = `url("${OTHER}/lab/pixel.svg?css=${method}")`;
    try {
      if (method === 'property') element.style.backgroundImage = value;
      if (method === 'setProperty') element.style.setProperty('background-image', value);
      if (method === 'cssText') element.style.cssText = `background-image: ${value}`;
      if (method === 'style-attribute') element.setAttribute('style', `background-image: ${value}`);
      if (method === 'style-forward') element.style = `background-image: ${value}`;
      if (method === 'style-text') { style.textContent = `#css-resource {background-image: ${value}}`; document.head.append(style); }
      if (method.startsWith('sheet-')) {
        const sheet = new CSSStyleSheet();
        if (method === 'sheet-replace') await sheet.replace(`#css-resource {background-image: ${value}}`);
        else sheet.replaceSync(`#css-resource {background-image: ${value}}`);
        document.adoptedStyleSheets = [...previousSheets, sheet];
      }
      const result = { image: getComputedStyle(element).backgroundImage, sameStyle: element.style === element.style, inline: element.style.getPropertyValue('background-image') };
      await sleep(150);
      return result;
    } finally { document.adoptedStyleSheets = previousSheets; style.remove(); element.remove(); }
  });
}
for (const kind of ['css-sri', 'css-sri-bad', 'css-hash', 'css-hash-bad']) {
  add(`policy-${kind}`, ['POLICY-01', 'SCRIPT-02'], () => useFrame(`/lab/policy?kind=${kind}`, frame => ({ applied: frame.contentWindow.getComputedStyle(frame.contentDocument.querySelector('.css-sri')).width === '21px' })), kind.startsWith('css-hash') ? { policyChange: 'csp-removed', proxyExpected: { applied: true } } : {});
}
add('sw-forged-rewrite-marker', ['SW-04', 'ESCAPE-01'], async () => {
  await installSW();
  return useFrame('/lab/sw-forged', async frame => {
    const image = frame.contentDocument.querySelector('img');
    if (!image.complete) await deadline(new Promise((resolve, reject) => { image.onload = resolve; image.onerror = reject; }));
    return { origin: frame.contentWindow.forgedOrigin, image: image.naturalWidth };
  });
});

add('script-local-globalthis-binding', ['SCRIPT-01'], () => {
  function read(globalThis) { return { local: globalThis, origin: location.origin }; }
  return read('application-value');
});
add('location-optional-chain-view', ['SCRIPT-01', 'ESCAPE-02'], () => ({ origin: window?.location?.origin, absent: null?.location?.origin ?? null }));
add('location-optional-chain-navigation', ['NAV-02', 'ESCAPE-02'], () => arrival(frame => frame.contentWindow?.location.assign(`${AUTH}/lab/arrival?optional=1`)));
add('location-invalid-protocol', ['NAV-02'], () => useFrame('/lab/arrival', frame => {
  const outcomes = [];
  for (const value of ['1bad', '?', '', Symbol('protocol')]) {
    try { frame.contentWindow.location.protocol = value; outcomes.push('accepted'); }
    catch (error) { outcomes.push(error.name); }
  }
  return outcomes;
}));
add('script-dynamic-inline-text', ['SCRIPT-01', 'ESCAPE-01'], async () => {
  const script = document.createElement('script');
  script.textContent = 'window.dynamicInlineOrigin = location.origin;';
  try { document.body.append(script); return window.dynamicInlineOrigin; }
  finally { script.remove(); delete window.dynamicInlineOrigin; }
});
add('css-style-text-node', ['SCRIPT-02', 'ESCAPE-01'], async () => {
  const style = document.createElement('style');
  const element = document.createElement('div');
  element.id = 'style-text-node';
  try {
    style.appendChild(document.createTextNode(`#style-text-node {background-image:url('${OTHER}/lab/pixel.svg?text-node=1')}`));
    document.head.append(style);
    document.body.append(element);
    const value = getComputedStyle(element).backgroundImage;
    await sleep(150);
    return value;
  } finally { style.remove(); element.remove(); }
});
add('worker-data-nested-worker', ['WORKER-01', 'ESCAPE-01'], () => {
  const inner = `data:text/javascript,${encodeURIComponent('onmessage=()=>postMessage("nested-data");')}`;
  const outer = `onmessage=()=>{const child=new Worker(${JSON.stringify(inner)});child.onmessage=e=>{postMessage(e.data);child.terminate();};child.onerror=()=>{postMessage("worker-error");child.terminate();};child.postMessage("go");};`;
  return useWorker(`data:text/javascript,${encodeURIComponent(outer)}`, {}, 'go');
});
add('worker-blob-module-absolute-import', ['WORKER-01', 'ESCAPE-01'], async () => {
  const url = URL.createObjectURL(new Blob([`import {value} from '${APP}/lab/value.js'; onmessage=()=>postMessage(value);`], { type: 'text/javascript' }));
  try { return await useWorker(url, { type: 'module' }, 'go'); }
  finally { URL.revokeObjectURL(url); }
});

add('script-dynamic-text-node-fragments', ['SCRIPT-01', 'ESCAPE-01'], () => {
  const script = document.createElement('script');
  const first = document.createTextNode('window.fragmentOrigin = loca');
  const second = document.createTextNode('tion.origin;');
  script.append(first, second);
  try {
    document.body.append(script);
    return { origin: window.fragmentOrigin, children: script.childNodes.length, first: first.parentNode === script, second: second.parentNode === script };
  } finally { script.remove(); delete window.fragmentOrigin; }
});
add('script-dynamic-source-reflection', ['SCRIPT-01'], () => {
  const script = document.createElement('script');
  const source = 'window.reflectedOrigin = location.origin;';
  script.text = source;
  try {
    document.body.append(script);
    return { origin: window.reflectedOrigin, source: script.text, content: script.textContent };
  } finally { script.remove(); delete window.reflectedOrigin; }
});

add('worker-blob-module-metadata', ['WORKER-01', 'SCRIPT-02'], async () => {
  const url = URL.createObjectURL(new Blob(['onmessage=()=>postMessage({same:import.meta.url===location.href,name:self.name});'], { type: 'text/javascript' }));
  try { return await useWorker(url, { type: 'module', name: 'blob-metadata' }, 'go'); }
  finally { URL.revokeObjectURL(url); }
});

for (const type of ['classic', 'module']) {
  add(`worker-blob-${type}-immediate-revoke-transfer`, ['WORKER-01'], async () => {
    const url = URL.createObjectURL(new Blob(['onmessage=e=>postMessage({size:e.data.byteLength,first:new Uint8Array(e.data)[0],trusted:e.isTrusted});'], { type: 'text/javascript' }));
    let worker;
    try {
      worker = new Worker(url, { type });
      URL.revokeObjectURL(url);
      const data = new Uint8Array([42, 1, 2]);
      const result = await deadline(new Promise((resolve, reject) => {
        worker.onmessage = event => resolve({ ...event.data, trustedReply: event.isTrusted });
        worker.onerror = event => reject(new Error(event.message || 'Worker failed'));
        worker.postMessage(data.buffer, [data.buffer]);
      }));
      return { ...result, detached: data.byteLength === 0 };
    } finally { worker?.terminate(); URL.revokeObjectURL(url); }
  });
  for (const kind of ['allow', 'connect', 'script', 'worker']) {
    add(`worker-blob-${type}-csp-${kind}`, ['WORKER-01', 'POLICY-01'], () => useFrame(`/lab/blob-policy?kind=${kind}&type=${type}`, frame => frame.contentWindow.blobPolicy()), { policyChange: 'csp-removed', proxyExpected: 'ready' });
  }
}

add('optional-chain-receiver-and-effects', ['SCRIPT-01'], () => {
  let reads = 0;
  let keys = 0;
  const object = { get owner() { reads++; return window; }, method() { return this === object; } };
  const missing = null;
  return {
    origin: object.owner?.location?.origin,
    receiver: object.method?.(),
    absent: missing?.[keys++] ?? null,
    reads,
    keys,
  };
});

add('worker-data-module-absolute-dependency', ['WORKER-01', 'ESCAPE-01'], () => {
  const source = `import {value} from '${APP}/lab/worker-dependency.js';onmessage=()=>postMessage(value);`;
  return useWorker(`data:text/javascript,${encodeURIComponent(source)}`, { type: 'module' }, 'go');
});
add('markup-addresses-read-while-parsing', ['ESCAPE-01'], () => useFrame('/lab/markup-addresses', frame => frame.contentWindow.markupAddresses));
add('policy-sync-xhr-forbidden', ['POLICY-01', 'SCRIPT-02'], () => useFrame('/lab/sync-xhr-policy', frame => frame.contentWindow.policyResult));
for (const type of ['classic', 'module']) {
  add(`shared-url-${type}-requests`, ['WORKER-01', 'ESCAPE-01'], async () => {
    const worker = new SharedWorker(`/lab/shared-worker.js?type=${type}`, { type, name: `requests-${type}` });
    try {
      return await deadline(new Promise((resolve, reject) => {
        worker.onerror = event => reject(new Error(event.message || 'SharedWorker failed'));
        worker.port.onmessage = event => resolve(event.data);
        worker.port.start();
        worker.port.postMessage('go');
      }));
    } finally { worker.port.close(); }
  });
  add(`shared-blob-${type}-boundary`, ['WORKER-01', 'ESCAPE-01'], async () => {
    const source = type === 'module'
      ? `import {value} from '${APP}/lab/worker-dependency.js';onconnect=e=>{const port=e.ports[0];port.onmessage=()=>port.postMessage(value);port.start();};`
      : 'onconnect=e=>{const port=e.ports[0];port.onmessage=()=>port.postMessage("shared-revoked");port.start();};';
    const url = URL.createObjectURL(new Blob([source], { type: 'text/javascript' }));
    let worker;
    try {
      worker = new SharedWorker(url, { type, name: `boundary-${type}` });
      if (type === 'classic') URL.revokeObjectURL(url);
      return await deadline(new Promise((resolve, reject) => {
        worker.onerror = event => reject(new Error(event.message || 'SharedWorker failed'));
        worker.port.onmessage = event => resolve(event.data);
        worker.port.start();
        worker.port.postMessage('go');
      }));
    } finally { worker?.port.close(); URL.revokeObjectURL(url); }
  });
}
