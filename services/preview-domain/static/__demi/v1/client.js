// The preview's client script, version 1: the first script of every preview document. It
// gives this origin's forwarder a channel to the Demi page (the top window) when the forwarder
// asks, gives the document's runtime one of its own, and sends a request the Demi page keeps
// through the target origin's boot page.
(() => {
  const BOOT = '/__demi/v1/boot.html';

  // A new channel to the Demi page for this document, for `purpose`.
  function open(purpose) {
    return new Promise((resolve, reject) => {
      const reply = new MessageChannel();
      const timer = setTimeout(() => reject(new Error('The Demi page did not answer')), 5000);
      reply.port1.onmessage = event => {
        clearTimeout(timer);
        reply.port1.close();
        resolve(event.ports[0]);
      };
      top.postMessage({ type: 'demi-preview-connect', purpose }, '*', [reply.port2]);
    });
  }

  // Hand a new channel to this origin's forwarder.
  async function connect() {
    const port = await open('forwarder');
    navigator.serviceWorker.controller.postMessage({ type: 'port' }, [port]);
  }

  let documentChannel;
  function channel() {
    documentChannel ??= open('document');
    return documentChannel;
  }

  // Navigate this frame with a request the Demi page keeps (a POST to another origin): the
  // target origin's boot page announces the token, and the forwarder there hands it on.
  // `frame`: the window to navigate, this document's by default (a blank frame's runtime passes its own).
  function navigateKept(url, request, frame = window) {
    return new Promise(resolve => {
      const reply = new MessageChannel();
      reply.port1.onmessage = event => {
        reply.port1.close();
        const target = url.pathname + url.search + url.hash;
        frame.location.href = `${url.origin}${BOOT}#token=${encodeURIComponent(event.data.token)}&to=${target}`;
        resolve();
      };
      top.postMessage({ type: 'demi-preview-keep', request }, '*', [reply.port2]);
    });
  }

  if (navigator.serviceWorker) {
    navigator.serviceWorker.addEventListener('message', event => {
      // A failed reconnection leaves the forwarder to time out; the next request asks again.
      if (event.data?.type === 'need-port') connect().catch(() => {});
    });
    navigator.serviceWorker.startMessages();
  }

  Object.defineProperty(window, '__demiPreview', { value: Object.freeze({ connect, channel, open, navigateKept }) });
})();
