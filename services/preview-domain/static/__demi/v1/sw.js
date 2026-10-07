// The preview forwarder, version 1 (docs/browser/preview.md § The forwarder and the relay).
// The same file for every Demi deployment: each request a preview document makes goes, as the browser made
// it, to the Demi page that embeds the preview, which answers it from the Host over the
// direct channel or the relay. It knows no Host, no site and no Demi version.

const PORT_WAIT_MS = 5000;
// Response headers a cross-origin CORS response shows a page without being listed (Fetch §
// CORS-safelisted response-header name).
const SAFELISTED = ['cache-control', 'content-language', 'content-length', 'content-type', 'expires', 'last-modified', 'pragma'];
const REDIRECTS = [301, 302, 303, 307, 308];

// The channel to the Demi page; lost when the browser stops this worker.
let port = null;
let portWaiters = [];
let nextId = 1;
// Requests waiting on the Demi page, by id: their head, then their body's chunks.
const pending = new Map();
// Navigations the boot page announced: target URL -> { token, referrer }.
const announced = new Map();

self.addEventListener('install', event => {
  event.waitUntil(self.skipWaiting());
});

self.addEventListener('activate', event => {
  event.waitUntil(self.clients.claim());
});

self.addEventListener('message', event => {
  const data = event.data;
  if (data?.type === 'port') {
    usePort(event.ports[0]);
  } else if (data?.type === 'announce') {
    announced.set(data.url, { token: data.token ?? null, referrer: data.referrer ?? '' });
    event.ports[0].postMessage('ok');
  }
});

self.addEventListener('fetch', event => {
  const url = new URL(event.request.url);
  // The forwarder's own files come from the preview domain; violation reports go to the Demi
  // deployment that set the policy.
  if (url.origin === self.location.origin && /^\/__demi\/v\d+\//.test(url.pathname)) return;
  if (event.request.destination === 'report') return;
  event.respondWith(forward(event));
});

// The preview domain's policy: which pages may embed this namespace. Kept in memory, not in
// this origin's storage, where the page would see it; asked again after the worker restarts.
let policy = null;
function frameAncestors() {
  policy ??= fetch('/__demi/v1/policy', { cache: 'no-store' }).then(response => response.json()).then(value => value.frameAncestors).catch(error => {
    policy = null;
    throw error;
  });
  return policy;
}

// What a page may read of a cross-origin CORS response: the browser does not filter a
// response a service worker gives, so the forwarder does, as the browser would.
function exposed(headers, credentials) {
  const listed = (headers.get('access-control-expose-headers') ?? '').split(',').map(name => name.trim().toLowerCase()).filter(Boolean);
  const all = listed.includes('*') && credentials !== 'include';
  const result = new Headers();
  for (const [name, value] of headers) {
    if (all ? name !== 'set-cookie' : SAFELISTED.includes(name) || listed.includes(name)) result.append(name, value);
  }
  return result;
}

// Every waiting request fails at once when its channel closes (the Demi page went away).
function failPending() {
  for (const waiter of pending.values()) waiter.fail();
  pending.clear();
}

function usePort(newPort) {
  if (port && port !== newPort) {
    failPending();
    port.close();
  }
  port = newPort;
  port.onmessage = event => pending.get(event.data.id)?.receive(event.data);
  port.addEventListener('close', () => {
    if (port !== newPort) return;
    port = null;
    failPending();
  });
  for (const resolve of portWaiters) resolve(port);
  portWaiters = [];
}

// A channel to the Demi page: the current one, or a new one asked of this origin's documents,
// whose client script gets it from the Demi page.
async function channel() {
  if (port) return port;
  const waiting = new Promise(resolve => portWaiters.push(resolve));
  const documents = await self.clients.matchAll({ type: 'window', includeUncontrolled: true });
  for (const client of documents) client.postMessage({ type: 'need-port' });
  let timer;
  const timeout = new Promise(resolve => { timer = setTimeout(() => resolve(null), PORT_WAIT_MS); });
  const result = await Promise.race([waiting, timeout]);
  clearTimeout(timer);
  return result;
}

// One request to the Demi page: resolves with its head; its body is pulled as the browser reads.
function exchange(target, message, transfer) {
  const id = nextId++;
  return new Promise(resolve => {
    let controller = null;
    const finish = () => pending.delete(id);
    pending.set(id, {
      receive(data) {
        if (data.type === 'head') {
          if (!data.body) finish();
          resolve({ ...data, body: data.body ? new ReadableStream({
            start(streamController) { controller = streamController; },
            pull() { target.postMessage({ type: 'pull', id }); },
            cancel() {
              finish();
              target.postMessage({ type: 'cancel', id });
            },
          }, { highWaterMark: 0 }) : null });
        } else if (data.type === 'chunk') {
          controller?.enqueue(new Uint8Array(data.bytes));
        } else if (data.type === 'end') {
          finish();
          controller?.close();
        } else if (data.type === 'fail') {
          finish();
          controller?.error(new TypeError('The preview connection failed'));
        }
      },
      fail() {
        resolve({ type: 'head', error: true });
        controller?.error(new TypeError('The preview connection closed'));
      },
    });
    target.postMessage({ ...message, id }, transfer);
  });
}

async function forward(event) {
  const request = event.request;
  // A preview opens only inside its Demi page: never as a page of its own.
  if (request.mode === 'navigate' && request.destination === 'document') {
    return refuse('A Demi preview opens only inside Demi.');
  }
  const target = await channel();
  if (!target) {
    if (request.mode === 'navigate') return reconnectPage();
    return Response.error();
  }
  const body = request.method === 'GET' || request.method === 'HEAD' ? null : await request.arrayBuffer();
  const announcement = request.mode === 'navigate' ? announced.get(request.url) : undefined;
  announced.delete(request.url);
  const head = await exchange(target, {
    type: 'fetch',
    url: request.url,
    method: request.method,
    headers: [...request.headers],
    body,
    mode: request.mode,
    destination: request.destination,
    credentials: request.credentials,
    redirect: request.redirect,
    referrer: request.referrer,
    referrerPolicy: request.referrerPolicy,
    token: announcement?.token ?? null,
    announcedReferrer: announcement?.referrer ?? null,
  }, body ? [body] : []);
  if (head.error) return Response.error();
  if (head.redirect && REDIRECTS.includes(head.status)) {
    return request.redirect === 'error' ? Response.error() : Response.redirect(head.redirect, head.status);
  }
  const crossOrigin = new URL(request.url).origin !== self.location.origin;
  // A cross-origin no-cors response reaches the page readable (it is never opaque, see
  // docs/browser/preview.md § The forwarder and the relay): it shows the headers a CORS request
  // without credentials would.
  const filter = { cors: request.credentials, 'no-cors': 'omit' }[request.mode];
  const headers = crossOrigin && filter ? exposed(new Headers(head.headers), filter) : new Headers(head.headers);
  if (request.mode === 'navigate') headers.append('content-security-policy', `frame-ancestors ${await frameAncestors()}`);
  const empty = [101, 204, 205, 304].includes(head.status) || request.method === 'HEAD';
  return new Response(empty ? null : head.body, { status: head.status, statusText: head.statusText, headers });
}

async function refuse(text) {
  return new Response(text, {
    status: 403,
    headers: { 'content-type': 'text/plain; charset=utf-8', 'content-security-policy': `frame-ancestors ${await frameAncestors()}` },
  });
}

// A navigation with no document left to ask for a channel: a page that connects, then loads again.
async function reconnectPage() {
  const html = '<!doctype html><meta charset="utf-8"><script src="/__demi/v1/client.js"></script>'
    + '<script>__demiPreview.connect().then(() => location.reload());</script>';
  return new Response(html, {
    headers: {
      'content-type': 'text/html; charset=utf-8',
      'content-security-policy': `frame-ancestors ${await frameAncestors()}`,
    },
  });
}
