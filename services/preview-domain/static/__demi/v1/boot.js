// The preview boot page's script, version 1: installs the forwarder on this origin, connects it to the
// Demi page, announces the navigation (its token, if the Demi page keeps the request, and the
// page that started it), then goes to the target. The target travels in the fragment, so the
// preview domain never sees it.
(async () => {
  const status = text => { document.body.textContent = text; };
  if (window.top === window) {
    status('A Demi preview opens only inside Demi.');
    return;
  }
  if (!navigator.serviceWorker) {
    status('This browser does not let the preview install its service worker here. Open Demi over HTTPS or on localhost.');
    return;
  }
  // `#to=<path>`, after an optional `token=<id>&`; without a fragment, this very address.
  const fragment = location.hash.slice(1);
  const token = fragment.match(/^token=([^&]*)&/)?.[1];
  const to = fragment.match(/(?:^|&)to=(.*)$/)?.[1];
  const target = new URL(to ?? location.pathname + location.search, location.origin).href;
  await navigator.serviceWorker.register('/__demi/v1/sw.js', { scope: '/' });
  await navigator.serviceWorker.ready;
  if (!navigator.serviceWorker.controller) {
    await new Promise(resolve => navigator.serviceWorker.addEventListener('controllerchange', resolve, { once: true }));
  }
  await __demiPreview.connect();
  const acknowledged = new MessageChannel();
  const announced = new Promise(resolve => { acknowledged.port1.onmessage = resolve; });
  navigator.serviceWorker.controller.postMessage({
    type: 'announce',
    url: target,
    token: token && decodeURIComponent(token),
    referrer: document.referrer,
  }, [acknowledged.port2]);
  await announced;
  acknowledged.port1.close();
  location.replace(target);
})();
