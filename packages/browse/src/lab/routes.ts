// @ts-nocheck: the spike's fixture routes, as the spike wrote them (`preview.md` § Tests).
import { readFile } from 'node:fs/promises';
import { transform } from 'esbuild';
import { createHash, randomUUID } from 'node:crypto';
import { gzipSync, brotliCompressSync } from 'node:zlib';

const directory = new URL('./fixtures/', import.meta.url);
const closingCounts = new WeakMap();
const binary = Buffer.from(Array.from({ length: 256 * 1024 }, (_, index) => index % 251));
const authorizationCodes = new Map();
const integrityCssOf = otherOrigin => `.css-sri{width:21px;background-image:url("${otherOrigin}/lab/pixel.svg")}`;

export async function handleLab(request, response, url, state, { appOrigin, authOrigin, otherOrigin }, localize) {
  if (!url.pathname.startsWith('/lab')) return false;
  const integrityCss = integrityCssOf(otherOrigin);
  const json = (value, status = 200, headers = {}) => {
    response.writeHead(status, { 'content-type': 'application/json', ...headers });
    response.end(JSON.stringify(value));
  };
  const html = (body, headers = {}) => {
    response.writeHead(200, { 'content-type': 'text/html; charset=utf-8', ...headers });
    response.end(`<!doctype html><html><head><meta charset="utf-8"><title>Behavior laboratory</title></head><body>${body}</body></html>`);
  };
  if (url.pathname.startsWith('/lab/bfcache-')) {
    response.setHeader('Cache-Control', 'private, max-age=60');
    const target = url.pathname.endsWith('-a') ? '/lab/bfcache-b' : '/lab/bfcache-a';
    html(`<h1 id="history-page">${url.pathname}</h1><a id="next-page" href="${target}">Next</a><script>window.pageShowPersisted=false;window.addEventListener('pageshow',event=>{window.pageShowPersisted=event.persisted;});</script>`);
  } else if (url.pathname === '/lab' || url.pathname === '/lab/deep/page') {
    html('<h1>Behavior laboratory</h1><div id="fixture"></div><script src="/lab/cases.js"></script><script src="/lab/scenarios.js"></script><script src="/lab/boundaries.js"></script><script src="/lab/csp-availability.js"></script><script src="/lab/generated-workers.js"></script><script src="/lab/lifecycle.js"></script>');
  } else if (['/lab/cases.js', '/lab/scenarios.js', '/lab/lifecycle.js', '/lab/callback.js', '/lab/boundaries.js', '/lab/csp-availability.js', '/lab/generated-workers.js', '/lab/isolation.js', '/lab/sw.js', '/lab/worker.js', '/lab/worker-module.js'].includes(url.pathname)) {
    response.writeHead(200, { 'content-type': 'text/javascript', 'service-worker-allowed': '/' });
    response.end(localize(await readFile(new URL(url.pathname.split('/').at(-1), directory), 'utf8')));
  } else if (url.pathname === '/lab/source-map.js' || url.pathname === '/lab/source-map.js.map') {
    const source = localize(await readFile(new URL('source-map-original.js', directory), 'utf8'));
    const result = await transform(source, { sourcemap: 'external', sourcefile: 'source-map-original.js', minify: true, format: 'esm' });
    const inline = url.searchParams.get('map') === 'inline';
    if (url.pathname.endsWith('.map')) {
      response.writeHead(200, { 'content-type': 'application/json' });
      response.end(result.map);
    } else {
      response.writeHead(200, { 'content-type': 'text/javascript' });
      response.end(result.code + (inline ? `//# sourceMappingURL=data:application/json;charset=utf-8;base64,${Buffer.from(result.map).toString('base64')}` : '//# sourceMappingURL=source-map.js.map'));
    }
  } else if (url.pathname === '/lab/source-map-original.js') {
    response.writeHead(200, { 'content-type': 'text/javascript' });
    response.end(localize(await readFile(new URL('source-map-original.js', directory), 'utf8')));
  } else if (url.pathname === '/lab/frame-host') {
    html(`<iframe id="child" src="${appOrigin}/lab"></iframe>`);
  } else if (url.pathname === '/lab/narrow/page') {
    html('<h1>Narrow scope page</h1>');
  } else if (url.pathname === '/lab/audio-worklet.js') {
    response.writeHead(200, { 'content-type': 'text/javascript', 'access-control-allow-origin': '*' });
    response.end('registerProcessor("fixture",class extends AudioWorkletProcessor{process(){return true;}});');
  } else if (url.pathname === '/lab/latin1') {
    response.writeHead(200, { 'content-type': 'text/html; charset=iso-8859-1' });
    response.end(Buffer.from('<html><body><h1 id="latin">café £</h1></body></html>', 'latin1'));
  } else if (url.pathname === '/lab/preload-page') {
    html(`<h1 id="preload">${request.headers['service-worker-navigation-preload'] ?? 'absent'}</h1>`);
  } else if (url.pathname === '/lab/large') {
    const bytes = Math.min(Number(url.searchParams.get('bytes') ?? 8388608), 67108864);
    response.writeHead(200, { 'content-type': 'application/octet-stream', 'content-length': bytes });
    let remaining = bytes;
    const send = () => {
      while (remaining > 0 && !response.destroyed) {
        const chunk = binary.subarray(0, Math.min(remaining, binary.length));
        remaining -= chunk.length;
        if (!response.write(chunk)) { response.once('drain', send); return; }
      }
      if (!response.destroyed) response.end();
    };
    send();
  } else if (url.pathname === '/lab/upload') {
    const hash = createHash('sha256');
    let bytes = 0;
    for await (const chunk of request) { hash.update(chunk); bytes += chunk.length; }
    json({ bytes, sha256: hash.digest('hex') });
  } else if (url.pathname === '/lab/echo') {
    response.setHeader('Access-Control-Allow-Origin', '*');
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    const body = Buffer.concat(chunks);
    json({ method: request.method, path: url.pathname + url.search, body: body.toString(), bytes: body.length, sha256: createHash('sha256').update(body).digest('hex'), headers: request.headers });
  } else if (url.pathname === '/lab/status') {
    const status = Number(url.searchParams.get('code'));
    response.writeHead(status, { 'content-type': 'text/plain', 'x-test-status': String(status) });
    response.end([204, 205, 304].includes(status) || request.method === 'HEAD' ? undefined : `status:${status}`);
  } else if (url.pathname === '/lab/bytes') {
    let body = binary;
    let status = 200;
    const headers = { 'content-type': 'application/octet-stream', 'accept-ranges': 'bytes', 'content-disposition': 'attachment; filename="fixture.bin"' };
    if (request.headers.range) {
      const match = /^bytes=(\d+)-(\d+)$/.exec(request.headers.range);
      if (!match) { response.writeHead(416); response.end(); return true; }
      const first = Number(match[1]);
      const last = Math.min(Number(match[2]), binary.length - 1);
      body = binary.subarray(first, last + 1);
      status = 206;
      headers['content-range'] = `bytes ${first}-${last}/${binary.length}`;
    }
    response.writeHead(status, { ...headers, 'content-length': body.length });
    response.end(body);
  } else if (url.pathname === '/lab/compressed') {
    const encoding = url.searchParams.get('encoding') ?? 'gzip';
    const body = Buffer.from('<!doctype html><html><body><h1 id="compressed">compressed 中文</h1></body></html>');
    response.writeHead(200, { 'content-type': 'text/html; charset=utf-8', 'content-encoding': encoding });
    response.end(encoding === 'br' ? brotliCompressSync(body) : gzipSync(body));
  } else if (url.pathname === '/lab/cache') {
    response.setHeader('Cache-Control', 'private, max-age=60');
    const variantEtag = `"fixture-${request.headers['x-flavor'] ?? 'default'}-v1"`;
    response.setHeader('ETag', variantEtag);
    response.setHeader('Vary', 'x-flavor');
    if (request.headers['if-none-match'] === variantEtag) { response.writeHead(304); response.end(); }
    else json({ flavor: request.headers['x-flavor'] ?? '', cookie: request.headers.cookie ?? '' });
  } else if (url.pathname === '/lab/cors') {
    const allow = url.searchParams.get('allow');
    if (allow) response.setHeader('Access-Control-Allow-Origin', allow);
    if (url.searchParams.has('credentials')) response.setHeader('Access-Control-Allow-Credentials', 'true');
    response.setHeader('Access-Control-Allow-Methods', 'GET, POST, PUT, OPTIONS');
    response.setHeader('Access-Control-Allow-Headers', 'x-probe, content-type, authorization');
    response.setHeader('Access-Control-Expose-Headers', 'x-public');
    response.setHeader('x-public', 'visible');
    response.setHeader('x-secret', 'hidden');
    json({ preflight: request.method === 'OPTIONS', cookie: request.headers.cookie ?? '', origin: request.headers.origin ?? '' });
  } else if (url.pathname === '/lab/cookie') {
    const cookies = url.searchParams.getAll('set');
    if (cookies.length) response.setHeader('Set-Cookie', cookies);
    json({ cookie: request.headers.cookie ?? '' });
  } else if (url.pathname === '/lab/redirect') {
    response.setHeader('Access-Control-Allow-Origin', '*');
    response.setHeader('Access-Control-Allow-Headers', 'authorization');
    if (request.method === 'OPTIONS') { response.writeHead(204); response.end(); return true; }
    const target = url.searchParams.get('to') ?? '/lab/echo';
    const code = Number(url.searchParams.get('code') ?? '302');
    response.writeHead(code, { location: target, 'set-cookie': 'chain=yes; Path=/; SameSite=Lax' });
    response.end();
  } else if (url.pathname === '/lab/arrival') {
    const data = { method: request.method, query: url.search, cookie: request.headers.cookie ?? '' };
    html(`<h1 id="arrival">arrived</h1><script>window.arrival=${JSON.stringify(data)};parent.postMessage({labArrival:true,...window.arrival,url:location.href},'*');</script>`);
  } else if (url.pathname === '/lab/refresh') {
    const target = `${authOrigin}/lab/arrival`;
    if (url.searchParams.get('kind') === 'header') html('refresh', { refresh: `0; url=${target}` });
    else html(`<meta http-equiv="refresh" content="0; url=${target}">`);
  } else if (url.pathname === '/lab/generated-dependency.js') {
    response.writeHead(200, { 'content-type': 'text/javascript', 'access-control-allow-origin': '*' });
    response.end(`export const origin=location.origin;export const ready=fetch('${otherOrigin}/lab/cors?allow=*').then(r=>r.json()).then(r=>r.origin);`);
  } else if (url.pathname === '/lab/csp-output') {
    const headers = Object.fromEntries(['content-security-policy', 'content-security-policy-report-only', 'x-content-security-policy', 'x-content-security-policy-report-only', 'x-webkit-csp'].map(name => [name, "default-src 'none'; report-uri /lab/echo"]));
    if (url.searchParams.get('type') === 'json') json({ value: 42 }, 200, headers);
    else if (url.searchParams.get('type') === 'worker') {
      response.writeHead(200, { ...headers, 'content-type': 'text/javascript' });
      response.end('onmessage=()=>postMessage(42);');
    } else html('<script>window.cspSwScriptRan=true;</script>', headers);
  } else if (url.pathname === '/lab/csp-worker.js') {
    response.writeHead(200, { 'content-type': 'text/javascript', 'content-security-policy': "connect-src 'none'" });
    response.end(`onmessage=async()=>{try{await fetch('${otherOrigin}/lab/cors?allow=*');postMessage('ready');}catch{postMessage('blocked');}};`);
  } else if (url.pathname === '/lab/csp-meta') {
    if (url.searchParams.get('kind') === 'static') {
      response.writeHead(200, { 'content-type': 'text/html; charset=utf-8' });
      response.end('<!doctype html><html><head><meta http-equiv="CoNtEnT-SeCuRiTy-PoLiCy" content="script-src &apos;none&apos;"><script>window.metaScriptRan=true;</script></head><body>Meta fixture</body></html>');
    }
    else html(`<script>
      window.tryMeta = kind => {
        let meta = document.createElement('meta');
        meta.content = "script-src 'none'";
        const name = 'Content-Security-Policy';
        if (kind.startsWith('connected-')) document.head.append(meta);
        if (kind === 'property' || kind === 'connected-property') meta.httpEquiv = name;
        else if (kind === 'attribute') meta.setAttribute('http-equiv', name);
        else if (kind === 'namespace') meta.setAttributeNS(null, 'http-equiv', name);
        else if (kind === 'inner-html') {
          const holder = document.createElement('div');
          holder.innerHTML = '<meta http-equiv="Content-Security-Policy" content="script-src &apos;none&apos;">';
          meta = holder.firstChild;
        } else if (kind.startsWith('parser-')) {
          const parsed = new DOMParser().parseFromString('<meta http-equiv="Content-Security-Policy" content="script-src &apos;none&apos;">', 'text/html');
          meta = document.adoptNode(parsed.querySelector('meta'));
        } else {
          meta.setAttribute('http-equiv', '');
          const attr = meta.getAttributeNode('http-equiv');
          if (kind === 'attr-node-value') attr.nodeValue = name;
          else if (kind === 'attr-text') attr.textContent = name;
          else if (kind === 'attribute-node' || kind === 'named-map') {
            const parsed = new DOMParser().parseFromString('<meta http-equiv="Content-Security-Policy">', 'text/html');
            const source = parsed.querySelector('meta');
            const detached = source.removeAttributeNode(source.getAttributeNode('http-equiv'));
            if (kind === 'attribute-node') meta.setAttributeNode(detached);
            else meta.attributes.setNamedItem(detached);
          } else attr.value = name;
        }
        if (kind === 'parser-before') document.head.lastChild.before(meta);
        else if (kind === 'parser-range') {
          const range = document.createRange();
          range.selectNodeContents(document.head);
          range.collapse(false);
          range.insertNode(meta);
        } else if (!meta.isConnected) document.head.append(meta);
        const script = document.createElement('script');
        script.textContent = 'window.metaScriptRan=true;';
        document.head.append(script);
        return window.metaScriptRan ?? false;
      };
    </script>`);
  } else if (url.pathname === '/lab/blob-policy') {
    const kind = url.searchParams.get('kind');
    const type = url.searchParams.get('type') === 'module' ? 'module' : 'classic';
    const policy = `script-src 'nonce-fixture' blob: ${kind === 'script' ? '' : "'self'"}; connect-src ${kind === 'connect' ? "'none'" : 'blob:'}; worker-src ${kind === 'worker' ? "'none'" : 'blob:'}`;
    html(`<script nonce="fixture">
      window.blobPolicy = async () => {
        const url = URL.createObjectURL(new Blob(['onmessage=()=>postMessage("ready");'], {type:'text/javascript'}));
        let worker;
        let timer;
        try {
          worker = new Worker(url, {type:${JSON.stringify(type)}});
          return await new Promise(resolve => {
            timer = setTimeout(() => resolve('timeout'), 5000);
            worker.onmessage = e => resolve(e.data);
            worker.onerror = e => {e.preventDefault(); resolve('blocked');};
            worker.postMessage('go');
          });
        } catch {return 'blocked';}
        finally {clearTimeout(timer); worker?.terminate(); URL.revokeObjectURL(url);}
      };
    </script>`, { 'content-security-policy': policy });
  } else if (url.pathname === '/lab/isolation') {
    const coep = url.searchParams.get('coep') ?? 'require-corp';
    html(`<h1 id="policy">Isolation fixture</h1><button id="popup">Open popup</button><script>window.isolationCoep=${JSON.stringify(coep)};window.isolationRemote=${JSON.stringify(authOrigin)};</script><script src="/lab/isolation.js"></script>`, {
      ...(coep === 'none' ? {} : {'cross-origin-embedder-policy':coep}),
      ...(url.searchParams.get('coop') === 'none' ? {} : {'cross-origin-opener-policy':'same-origin'}),
      ...(url.searchParams.has('dip') ? {'document-isolation-policy':url.searchParams.get('dip')} : {}),
    });
  } else if (url.pathname === '/lab/isolation-sw.js') {
    const document = `<h1 id="policy">SW isolation fixture</h1><button id="popup">Open popup</button><script>window.isolationCoep='require-corp';window.isolationRemote=${JSON.stringify(authOrigin)};</script><script src="/lab/isolation.js"></script>`;
    response.writeHead(200, {'content-type':'text/javascript','service-worker-allowed':'/'});
    response.end(`self.addEventListener('install',event=>event.waitUntil(self.skipWaiting()));self.addEventListener('activate',event=>event.waitUntil(self.clients.claim()));self.addEventListener('fetch',event=>{if(new URL(event.request.url).pathname==='/lab/isolation-sw-page')event.respondWith(new Response(${JSON.stringify(document)},{headers:{'content-type':'text/html','cross-origin-opener-policy':'same-origin','cross-origin-embedder-policy':'require-corp'}}));});`);
  } else if (url.pathname === '/lab/isolation-callback') {
    html(`<script>if(opener)opener.postMessage({popup:true,origin:location.origin},'*');</script>`);
  } else if (url.pathname === '/lab/isolation-pixel') {
    response.writeHead(200, {'content-type':'image/svg+xml', ...(url.searchParams.has('corp') ? {'cross-origin-resource-policy':'cross-origin'} : {})});
    response.end('<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><rect width="1" height="1"/></svg>');
  } else if (url.pathname === '/lab/policy') {
    const kind = url.searchParams.get('kind');
    if (kind === 'xfo') html('<h1 id="policy">xfo</h1>', { 'x-frame-options': 'DENY' });
    else if (kind === 'ancestors') html('<h1 id="policy">ancestors</h1>', { 'content-security-policy': "frame-ancestors 'none'" });
    else if (kind === 'csp') html('<script nonce="fixture">window.policyAllowed=true;</script><script>window.policyForbidden=true;</script>', { 'content-security-policy': "script-src 'nonce-fixture'; object-src 'none'; base-uri 'none'" });
    else if (kind === 'csp-hash' || kind === 'csp-hash-bad' || kind === 'csp-inline' || kind === 'csp-multiple') {
      const script = 'window.integrityWorked = location.origin;';
      const hash = createHash('sha256').update(script).digest('base64');
      const policy = kind === 'csp-inline' ? "script-src 'unsafe-inline'" : `script-src 'sha256-${kind === 'csp-hash-bad' ? 'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=' : hash}'`;
      html(`<script>${script}</script><script>window.policyForbidden=true;</script>`, { 'content-security-policy': kind === 'csp-multiple' ? [policy, "script-src 'none'"] : policy });
    }
    else if (kind === 'csp-origin') html(`<script src="${authOrigin}/lab/integrity.js"></script>`, { 'content-security-policy': `script-src ${authOrigin}` });
    else if (kind === 'sri-cross' || kind === 'sri-cross-bad') {
      const integrity = createHash('sha256').update('window.integrityWorked = location.origin;').digest('base64');
      html(`<script crossorigin="anonymous" src="${authOrigin}/lab/integrity.js" integrity="sha256-${kind === 'sri-cross-bad' ? 'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=' : integrity}"></script>`);
    } else if (kind === 'sri') {
      const script = "window.integrityWorked = location.origin;";
      const integrity = createHash('sha256').update(script).digest('base64');
      html(`<script src="/lab/integrity.js" integrity="sha256-${url.searchParams.has('bad') ? 'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=' : integrity}"></script>`);
    } else if (['css-sri', 'css-sri-bad', 'css-hash', 'css-hash-bad'].includes(kind)) {
      const hash = kind.endsWith('-bad') ? 'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=' : createHash('sha256').update(integrityCss).digest('base64');
      if (kind.startsWith('css-sri')) html(`<link rel="stylesheet" href="/lab/integrity.css" integrity="sha256-${hash}"><div class="css-sri">CSS policy</div>`);
      else html(`<style>${integrityCss}</style><div class="css-sri">CSS policy</div>`, { 'content-security-policy': `style-src 'sha256-${hash}'` });
    } else if (kind === 'coop') html('<script>window.policyIsolated=crossOriginIsolated;</script>', { 'cross-origin-opener-policy': 'same-origin', 'cross-origin-embedder-policy': 'require-corp', ...(url.searchParams.has('corp') ? { 'cross-origin-resource-policy': 'cross-origin' } : {}), ...(url.searchParams.has('dip') ? { 'document-isolation-policy': 'isolate-and-require-corp' } : {}) });
    else if (kind === 'tt') html('<script nonce="fixture">try{document.body.innerHTML="blocked";window.ttBlocked=false;}catch(e){window.ttBlocked=true;}</script>', { 'content-security-policy': "require-trusted-types-for 'script'; script-src 'nonce-fixture'" });
    else html('unknown policy');
  } else if (url.pathname === '/lab/integrity.js') {
    response.writeHead(200, { 'content-type': 'text/javascript', 'access-control-allow-origin': '*' });
    response.end('window.integrityWorked = location.origin;');
  } else if (url.pathname === '/lab/modules/chain.js') {
    response.writeHead(200, { 'content-type': 'text/javascript' });
    response.end(`import {url as importedURL} from '${appOrigin}/lab/value.js'; export {importedURL}; export {value as named} from '${appOrigin}/lab/value.js'; export * from '${appOrigin}/lab/value.js'; export const relative = () => import('../value.js?from=relative');`);
  } else if (url.pathname === '/lab/import-map-page') {
    const imports = { fixture: `${appOrigin}/lab/value.js?source=default`, 'pkg/': `${appOrigin}/lab/`, [`${appOrigin}/lab/alias.js`]: `${appOrigin}/lab/value.js?source=absolute` };
    const scopes = { [`${appOrigin}/lab/modules/`]: { fixture: `${appOrigin}/lab/value.js?source=scope` } };
    html(`<script type="importmap">${JSON.stringify({ imports, scopes })}</script><script type="module" src="/lab/modules/map-entry.js"></script>`);
  } else if (url.pathname === '/lab/modules/map-entry.js') {
    response.writeHead(200, { 'content-type': 'text/javascript' });
    response.end(`import {url as scoped} from 'fixture'; import {url as prefixed} from 'pkg/value.js'; import {url as absolute} from '${appOrigin}/lab/alias.js'; window.moduleMapResult={scoped,prefixed,absolute};`);
  } else if (url.pathname === '/lab/worker-import.js') {
    response.writeHead(200, { 'content-type': 'text/javascript' });
    response.end(`importScripts('${appOrigin}/lab/imported-classic.js'); onmessage=()=>postMessage({value:self.importedClassic,url:location.href});`);
  } else if (url.pathname === '/lab/sync-xhr-policy') {
    // A page that forbids synchronous requests, as Cloudflare's challenge frames do, and runs a
    // script from a blob: address.
    html(`<script>window.policyResult=new Promise(resolve=>{let refused;try{const request=new XMLHttpRequest();request.open('GET','/lab/value.js',false);request.send();refused='sent';}catch(error){refused=error.name;}const script=document.createElement('script');script.src=URL.createObjectURL(new Blob(['window.blobHref=location.href;'],{type:'text/javascript'}));script.onload=()=>resolve({refused,blobHref:window.blobHref});script.onerror=()=>resolve({refused,blobHref:'load-error'});document.head.append(script);});</script>`, { 'permissions-policy': 'sync-xhr=()' });
  } else if (url.pathname === '/lab/local-network-probe') {
    const targets = [`${appOrigin}/lab/value.js`, `${otherOrigin}/lab/value.js`, `https://${url.host}/lab/value.js`];
    html(`<script>Promise.all(${JSON.stringify(targets)}.map(target=>fetch(target,{mode:'no-cors'}).then(()=>'loaded',error=>error.name))).then(localNetwork=>parent.postMessage({localNetwork},'*'));</script>`);
  } else if (url.pathname === '/lab/closing-connection') {
    // Every second request on a connection finds it closed, with no response: the server ended
    // the idle connection just as the client reused it.
    const count = (closingCounts.get(request.socket) ?? 0) + 1;
    closingCounts.set(request.socket, count);
    if (count % 2 === 0) request.socket.destroy();
    else json({ count });
  } else if (url.pathname === '/lab/markup-addresses') {
    // Addresses written in the markup, read back while the document is still parsing: Turnstile
    // finds its own script tag this way.
    html(`<script src="${otherOrigin}/lab/value.js" type="module"></script><img src="${otherOrigin}/lab/pixel.svg"><script>window.markupAddresses=[document.scripts[0].src,document.images[0].src];</script>`);
  } else if (url.pathname === '/lab/shared-worker.js') {
    // A site's shared worker, as chat and mail clients have: it imports and fetches from other origins.
    const module = url.searchParams.get('type') === 'module';
    response.writeHead(200, { 'content-type': 'text/javascript' });
    response.end(`${module ? `import {value} from '${appOrigin}/lab/worker-dependency.js';` : `importScripts('${appOrigin}/lab/imported-classic.js');const value=self.importedClassic;`}onconnect=e=>{const port=e.ports[0];port.onmessage=async()=>port.postMessage({value,url:location.href,remote:(await(await fetch('${otherOrigin}/lab/cors?allow=*')).json()).origin});port.start();};`);
  } else if (url.pathname === '/lab/imported-classic.js') {
    response.writeHead(200, { 'content-type': 'text/javascript' });
    response.end('self.importedClassic="loaded";');
  } else if (url.pathname === '/lab/worker-dependency.js') {
    response.writeHead(200, { 'content-type': 'text/javascript', 'access-control-allow-origin': '*' });
    response.end('export const value="worker-dependency";');
  } else if (url.pathname === '/lab/value.js') {
    response.writeHead(200, { 'content-type': 'text/javascript' });
    response.end(`export const value = 'module-value'; export const url = import.meta.url;`);
  } else if (url.pathname === '/lab/integrity.css') {
    response.writeHead(200, { 'content-type': 'text/css', 'access-control-allow-origin': '*' });
    response.end(integrityCss);
  } else if (url.pathname === '/lab/style.css') {
    response.writeHead(200, { 'content-type': 'text/css' });
    response.end(`.probe-image { background-image: url('${otherOrigin}/lab/pixel.svg'); }`);
  } else if (url.pathname === '/lab/pixel.svg') {
    response.writeHead(200, { 'content-type': 'image/svg+xml' });
    response.end('<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><rect width="2" height="2" fill="red"/></svg>');
  } else if (url.pathname === '/lab/stream') {
    response.writeHead(200, { 'content-type': 'text/plain' });
    response.flushHeaders();
    state.activeStreams++;
    let index = 0;
    const timer = setInterval(() => response.write(`chunk-${++index}\n`), 40);
    response.once('close', () => { clearInterval(timer); state.activeStreams--; });
  } else if (url.pathname === '/lab/write') {
    for await (const chunk of request) { /* Consume the explicit fixture write body before committing. */ }
    const id = url.searchParams.get('id');
    state.writes.set(id, (state.writes.get(id) ?? 0) + 1);
    const timer = setTimeout(() => json({ committed: true, count: state.writes.get(id) }), 1500);
    response.once('close', () => clearTimeout(timer));
  } else if (url.pathname === '/lab/clear') {
    response.writeHead(200, { 'content-type': 'text/plain', 'clear-site-data': '"cache", "cookies", "storage"' });
    response.end('cleared');
  } else if (url.pathname === '/lab/authorize') {
    response.setHeader('Access-Control-Allow-Origin', '*');
    if (url.searchParams.get('redirect_uri') !== `${appOrigin}/lab/callback`) {
      json({ error: 'invalid_redirect_uri' }, 400);
      return true;
    }
    for (const [code, grant] of authorizationCodes) if (grant.expires < Date.now()) authorizationCodes.delete(code);
    const code = randomUUID();
    authorizationCodes.set(code, { challenge: url.searchParams.get('code_challenge'), redirect: url.searchParams.get('redirect_uri'), expires: Date.now() + 60000 });
    const parameters = new URLSearchParams({ code, state: url.searchParams.get('state') });
    const mode = url.searchParams.get('mode');
    if (mode === 'json') json({ code });
    else if (mode === 'post') html(`<form method="post" action="${appOrigin}/lab/callback"><input name="code" value="${code}"><input name="state" value="${url.searchParams.get('state')}"></form><script>document.forms[0].submit();</script>`);
    else {
      response.writeHead(302, { location: `${appOrigin}/lab/callback${mode === 'fragment' ? '#' : '?'}${parameters}`, 'set-cookie': 'fixture-login=ok; Secure; HttpOnly; SameSite=None; Path=/' });
      response.end();
    }
  } else if (url.pathname === '/lab/token') {
    response.setHeader('Access-Control-Allow-Origin', '*');
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    const body = new URLSearchParams(Buffer.concat(chunks).toString());
    const grant = authorizationCodes.get(body.get('code'));
    const challenge = createHash('sha256').update(body.get('code_verifier') ?? '').digest('base64url');
    if (!grant || grant.expires < Date.now() || grant.challenge !== challenge || grant.redirect !== body.get('redirect_uri')) json({ error: 'invalid_grant' }, 400);
    else {
      authorizationCodes.delete(body.get('code'));
      json({ access_token: 'fixture-token', token_type: 'Bearer' });
    }
  } else if (url.pathname === '/lab/callback') {
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    const body = request.method === 'POST' ? Object.fromEntries(new URLSearchParams(Buffer.concat(chunks).toString())) : null;
    html(`<pre id="auth-result">Authorizing…</pre>${body ? `<script>window.callbackPayload=${JSON.stringify(body)};</script>` : ''}<script src="/lab/callback.js"></script>`);
  } else if (url.pathname === '/lab/sw-invalid.js') {
    response.writeHead(200, { 'content-type': 'text/plain' });
    response.end('not a service worker');
  } else if (url.pathname === '/lab/sw-failed.js') {
    response.writeHead(200, { 'content-type': 'text/javascript', 'service-worker-allowed': '/' });
    response.end("self.addEventListener('install',event=>event.waitUntil(Promise.reject(new Error('fixture install failure'))));");
  } else {
    response.writeHead(404);
    response.end('Unknown laboratory route');
  }
  return true;
}
