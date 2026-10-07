(async () => {
  const payload = window.callbackPayload ?? Object.fromEntries(new URLSearchParams(location.search || location.hash.slice(1)));
  const saved = JSON.parse(sessionStorage.getItem('auth-fixture') ?? '{}');
  const fixtureAuth = { stateValid: saved.state === payload.state };
  if (fixtureAuth.stateValid) {
    const body = new URLSearchParams({ code: payload.code, code_verifier: saved.verifier, redirect_uri: 'http://localhost:19401/lab/callback' });
    const response = await fetch('https://auth.upstream.test:19444/lab/token', { method: 'POST', body });
    fixtureAuth.status = response.status;
    fixtureAuth.token = await response.json();
    const replay = await fetch('https://auth.upstream.test:19444/lab/token', { method: 'POST', body });
    fixtureAuth.replayStatus = replay.status;
  }
  window.authResult = fixtureAuth;
  document.querySelector('#auth-result').textContent = JSON.stringify(fixtureAuth);
  if (window.opener) { window.opener.postMessage({ fixtureAuth }, 'http://localhost:19401'); window.close(); }
})().catch(error => { window.authResult = { error: error.message }; });
