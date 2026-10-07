const cspHeaders = ['content-security-policy', 'content-security-policy-report-only', 'x-content-security-policy', 'x-content-security-policy-report-only', 'x-webkit-csp'];
for (const type of ['html', 'json', 'worker']) {
  add(`csp-response-headers-${type}`, ['POLICY-01'], async () => {
    const response = await fetch(`/lab/csp-output?type=${type}`);
    return cspHeaders.map(name => response.headers.has(name));
  }, { policyChange: 'csp-removed', directExpected: cspHeaders.map(() => true), proxyExpected: cspHeaders.map(() => false) });
}
add('csp-static-meta', ['POLICY-01'], () => useFrame('/lab/csp-meta?kind=static', frame => frame.contentWindow.metaScriptRan ?? false), { policyChange: 'csp-removed', directExpected: false, proxyExpected: true });
for (const kind of ['property', 'attribute', 'namespace', 'attr-value', 'attr-node-value', 'attr-text', 'attribute-node', 'named-map', 'inner-html', 'parser-insert', 'parser-before', 'parser-range', 'connected-property', 'connected-attr']) {
  add(`csp-dynamic-meta-${kind}`, ['POLICY-01'], () => useFrame('/lab/csp-meta', frame => frame.contentWindow.tryMeta(kind)), { policyChange: 'csp-removed', directExpected: false, proxyExpected: true });
}
for (const kind of ['synthetic', 'cached', 'network-clone', 'json']) {
  add(`csp-sw-response-${kind}`, ['POLICY-01', 'SW-04'], async () => {
    await installSW();
    const url = `/lab/sw-csp?kind=${kind}`;
    const response = await fetch(url);
    const headers = cspHeaders.map(name => response.headers.has(name));
    if (kind === 'json') return { headers, body: await response.json() };
    return useFrame(url, frame => ({ headers, script: frame.contentWindow.cspSwScriptRan ?? false }));
  }, { policyChange: 'csp-removed', directExpected: { headers: cspHeaders.map(() => true), ...(kind === 'json' ? { body: { value: 42 } } : { script: false }) }, proxyExpected: { headers: cspHeaders.map(() => false), ...(kind === 'json' ? { body: { value: 42 } } : { script: true }) } });
}
add('csp-worker-response-policy', ['POLICY-01', 'WORKER-01'], () => useWorker('/lab/csp-worker.js', {}, 'go'), { policyChange: 'csp-removed', directExpected: 'blocked', proxyExpected: 'ready' });
