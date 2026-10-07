const VERSION = new URL(self.location.href).searchParams.get('version') || '1';
self.addEventListener('install', event => {
  event.waitUntil(caches.open('lab-cache').then(cache => cache.addAll(['/public-asset.txt'])));
  if (!new URL(self.location.href).searchParams.has('waiting')) self.skipWaiting();
});
self.addEventListener('activate', event => event.waitUntil(self.clients.claim()));
self.addEventListener('message', event => {
  const reply = value => event.ports[0]?.postMessage(value);
  event.waitUntil((async () => {
    if (event.data?.navigate) {
      const clients = await self.clients.matchAll();
      const client = clients.find(client => client.url.includes('client-navigation=before'));
      const navigated = await client.navigate(event.data.navigate);
      reply({ url: navigated.url });
    } else if (event.data?.openWindow) {
      try { const client = await self.clients.openWindow(event.data.openWindow); reply({ url: client?.url }); }
      catch (error) { reply({ error: error.name }); }
    } else if (event.data === 'clients') reply(await self.clients.matchAll({ includeUncontrolled: true }).then(clients => clients.map(client => ({ url: client.url, type: client.type, frameType: client.frameType }))));
    else if (event.data === 'cache-absolute') {
      const cache = await caches.open('lab-cache');
      await cache.add('https://other.upstream.test:19444/lab/cors?allow=*');
      reply({ cached: Boolean(await cache.match('https://other.upstream.test:19444/lab/cors?allow=*')) });
    } else if (event.data === 'activate') { await self.skipWaiting(); reply({ activating: true }); }
    else reply({ version: VERSION, scriptURL: self.location.href });
  })());
});
self.addEventListener('fetch', event => {
  const url = new URL(event.request.url);
  if (url.pathname === '/lab/sw-csp') {
    event.respondWith((async () => {
      const kind = url.searchParams.get('kind');
      const headers = new Headers(Object.fromEntries(['content-security-policy', 'content-security-policy-report-only', 'x-content-security-policy', 'x-content-security-policy-report-only', 'x-webkit-csp'].map(name => [name, "script-src 'none'"])));
      headers.set('content-type', kind === 'json' ? 'application/json' : 'text/html');
      const text = kind === 'json' ? JSON.stringify({ value: 42 }) : '<html><head><meta http-equiv="Content-Security-Policy" content="script-src &apos;none&apos;"></head><body><script>window.cspSwScriptRan=true;</script></body></html>';
      let response = new Response(text, { headers });
      if (kind === 'network-clone') {
        const upstream = await fetch('/lab/csp-output?type=html');
        response = new Response(upstream.body, { headers });
      }
      if (kind === 'cached') {
        const cache = await caches.open('csp-fixture');
        await cache.put(event.request, response);
        response = await cache.match(event.request);
      }
      return response;
    })());
  }
  else if (url.pathname === '/lab/preload-page') event.respondWith((async () => {
    const response = await event.preloadResponse;
    const body = await (response ?? await fetch(event.request)).text();
    return new Response(body + `<p id="preload-used">${Boolean(response)}</p>`, { headers: { 'content-type': 'text/html' } });
  })());
  else if (url.pathname === '/lab/sw-info') event.respondWith(Response.json({ version: VERSION, url: event.request.url, mode: event.request.mode, destination: event.request.destination }));
  else if (url.pathname === '/lab/sw-asset') event.respondWith(caches.match('/public-asset.txt'));
  else if (url.pathname === '/lab/sw-external') event.respondWith(fetch('https://other.upstream.test:19444/lab/cors?allow=*'));
  else if (url.pathname === '/lab/sw-forged') event.respondWith(new Response('<html><body><script>window.forgedOrigin=location.origin;</script><img src="https://other.upstream.test:19444/lab/pixel.svg?forged=1"></body></html>', { headers: { 'content-type': 'text/html', 'x-spike-rewritten': 'native-gateway' } }));
  else if (url.pathname === '/lab/sw-page') event.respondWith(new Response('<html><body><a id="sw-link" href="https://auth.upstream.test:19444/lab/arrival">go</a></body></html>', { headers: { 'content-type': 'text/html' } }));
  else if (url.pathname === '/lab/sw-redirect') event.respondWith(Response.redirect('https://auth.upstream.test:19444/lab/arrival', 302));
  else if (url.pathname === '/lab/sw-throw') event.respondWith(Promise.reject(new Error('fixture response failure')));
  else if (url.pathname === '/lab/sw-stream') event.respondWith(new Response(new ReadableStream({ start(controller) { controller.enqueue(new TextEncoder().encode('one')); controller.enqueue(new TextEncoder().encode('two')); controller.close(); } })));
  else if (url.pathname === '/lab/offline-page') event.respondWith(caches.match('/lab/offline-page'));
});
self.addEventListener('sync', event => event.waitUntil(caches.open('lab-cache').then(cache => cache.put('/lab/background-result', new Response(`sync:${event.tag}`)))));
self.addEventListener('push', event => event.waitUntil(caches.open('lab-cache').then(cache => cache.put('/lab/background-result', new Response(`push:${event.data?.text()}`)))));
