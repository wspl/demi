window.lifecycle = {
  async install(version = '1') { await installSW(`?version=${version}`); return this.info(); },
  async info() { return { controller: navigator.serviceWorker.controller?.scriptURL ?? null, reply: await swMessage('version') }; },
  async cached() { return await (await fetch('/lab/sw-asset')).text(); },
  async prepareCache() {
    await installSW('?version=1');
    const cache = await caches.open('lab-cache');
    await cache.put('/lab/offline-page', new Response('<html><body><h1 id="offline-page">offline navigation</h1><script>window.offlineScript=location.origin;<\/script></body></html>', { headers: { 'content-type': 'text/html' } }));
    return await this.cached();
  },
  async updateWaiting() {
    const registration = await navigator.serviceWorker.register('/lab/sw.js?version=2&waiting=1', { scope: '/' });
    await deadline((async () => { while (!registration.waiting) await sleep(20); })());
    return { waiting: registration.waiting.scriptURL, active: registration.active.scriptURL, current: await swMessage('version') };
  },
  async activateWaiting() {
    const registration = await navigator.serviceWorker.getRegistration();
    await swMessage('activate', registration.waiting);
    await deadline((async () => { while (!(navigator.serviceWorker.controller?.scriptURL.includes('version=2'))) await sleep(20); })());
    return this.info();
  },
  async unregister() { const registration = await navigator.serviceWorker.getRegistration(); return registration?.unregister(); },
  async clear() { localStorage.setItem('clear-target', 'present'); await fetch('/lab/clear'); return true; },
  storage() { return { local: localStorage.getItem('clear-target'), cookie: document.cookie }; },
  async syncRegister() {
    const registration = await installSW('?version=1');
    try { await registration.sync.register('lab-sync'); return { registered: true, tags: await registration.sync.getTags() }; }
    catch (error) { return { registered: false, error: error.name }; }
  },
  async backgroundResult() { return (await caches.match('/lab/background-result'))?.text() ?? null; },
  async passkey() {
    try {
      const credential = await navigator.credentials.create({ publicKey: {
        challenge: new Uint8Array(32).fill(7),
        rp: { id: 'localhost', name: 'Fixture RP' },
        user: { id: new Uint8Array([1, 2, 3]), name: 'fixture', displayName: 'Fixture' },
        pubKeyCredParams: [{ type: 'public-key', alg: -7 }],
        authenticatorSelection: { userVerification: 'required' },
        timeout: 4000,
      } });
      return { created: Boolean(credential), type: credential?.type };
    } catch (error) { return { created: false, error: error.name }; }
  },
  async authPrepare(mode = 'get') {
    const verifier = crypto.randomUUID() + crypto.randomUUID();
    const state = crypto.randomUUID();
    const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(verifier)));
    const challenge = btoa(String.fromCharCode(...digest)).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '');
    sessionStorage.setItem('auth-fixture', JSON.stringify({ verifier, state }));
    const url = new URL(`${AUTH}/lab/authorize`);
    url.search = new URLSearchParams({ redirect_uri: `${APP}/lab/callback`, state, code_challenge: challenge, mode }).toString();
    window.authTarget = url.href;
    window.authMode = mode;
    const button = document.createElement('button');
    button.id = 'begin-auth';
    button.textContent = 'Begin fixture login';
    button.onclick = () => {
      if (mode === 'popup') {
        window.authPopup = window.open(window.authTarget, 'fixture-login', 'width=500,height=600');
        window.addEventListener('message', event => {
          if (event.source === window.authPopup && event.origin === APP && event.data?.fixtureAuth) {
            window.authResult = event.data.fixtureAuth;
          }
        });
      } else location.assign(window.authTarget);
    };
    document.body.append(button);
    return { prepared: true };
  },
  async authNegative(kind) {
    if (kind === 'redirect') return { status: (await fetch(`${AUTH}/lab/authorize?redirect_uri=https%3A%2F%2Finvalid.example%2Fcallback&state=s&code_challenge=x`)).status };
    const authorization = await fetch(`${AUTH}/lab/authorize?redirect_uri=${encodeURIComponent(`${APP}/lab/callback`)}&state=s&code_challenge=x&mode=json`);
    const data = await authorization.json();
    const response = await fetch(`${AUTH}/lab/token`, { method: 'POST', body: new URLSearchParams({ code: data.code, code_verifier: 'wrong', redirect_uri: `${APP}/lab/callback` }) });
    return { status: response.status, body: await response.json() };
  },
};
window.labReady = true;

window.lifecycle.position = () => new Promise((resolve,reject) => {
  navigator.geolocation.getCurrentPosition(position=>resolve({latitude:position.coords.latitude,longitude:position.coords.longitude}),error=>reject(new Error(error.message)),{timeout:3000});
});
