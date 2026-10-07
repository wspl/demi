// The page's own blank frames (`docs/browser/preview.md` § Storage and browser features). A frame without an
// address or with about:blank shares the page's origin, so the page reaches its window and
// document as natively; but nothing ran the runtime there, and Chrome does not let the forwarder
// control it, so its requests would go straight to the preview domain. When the page first
// reaches such a frame's window, the runtime installs itself there, from its own source, and
// gives the frame the page's network functions, which go through the forwarder. Loads that
// elements inside the blank document start themselves stay uncontrolled.
// A frame that loads another preview origin goes through that origin's bootstrap first, which
// replaces itself with the page: the frame's first load event is the bootstrap's. The page
// sees one load, the page's own, as natively.
function installBootstrapLoads(isBootstrap) {
  const nativeSource = Object.getOwnPropertyDescriptor(Element.prototype, 'getAttribute').value;
  // Frames whose bootstrap has loaded and whose page is on its way.
  const booted = new WeakSet();
  // A load event never reaches the window (DOM § get the parent of a Document): the document
  // is the first place to catch it.
  document.addEventListener('load', event => {
    const frame = event.target;
    if (!(frame instanceof HTMLIFrameElement)) return;
    if (booted.has(frame)) {
      booted.delete(frame);
      return;
    }
    if (!isBootstrap(nativeSource.call(frame, 'src') ?? '')) return;
    booted.add(frame);
    event.stopImmediatePropagation();
  }, true);
}

// `blankBoot` gives the boot data of a blank frame of this document.
export function installFrameRuntime(isBootstrap, blankBoot) {
  installBootstrapLoads(isBootstrap);
  const nativeToString = Function.prototype.toString;
  const contentWindow = Object.getOwnPropertyDescriptor(HTMLIFrameElement.prototype, 'contentWindow');
  const contentDocument = Object.getOwnPropertyDescriptor(HTMLIFrameElement.prototype, 'contentDocument');
  const prepared = new WeakSet();

  function blank(frame) {
    if (frame.hasAttribute('srcdoc')) return false;
    const address = (frame.getAttribute('src') ?? '').trim();
    return address === '' || address === 'about:blank';
  }

  function prepare(frame) {
    if (!frame.isConnected || !blank(frame)) return;
    const child = contentWindow.get.call(frame);
    // A frame that navigated away is another origin's, and controlled again.
    if (!child || prepared.has(child)) return;
    try {
      if (child.location.href !== 'about:blank') return;
    } catch {
      return;
    }
    prepared.add(child);
    child.__proxyBoot = blankBoot();
    // The frame's runtime gets a channel of its own to the Demi page (one per document).
    const preview = globalThis.__demiPreview;
    Object.defineProperty(child, '__demiPreview', { value: Object.freeze({ ...preview, channel: () => preview.open('document') }) });
    // The frame's own eval, before anything replaced it; the source is the runtime's own.
    child.eval(`(${nativeToString.call(globalThis.__proxyRuntimeFunction)})()`);
    for (const name of ['fetch', 'XMLHttpRequest', 'WebSocket', 'EventSource', 'Request', 'Worker']) {
      if (globalThis[name]) child[name] = globalThis[name];
    }
    if (child.navigator?.sendBeacon) child.navigator.sendBeacon = navigator.sendBeacon.bind(navigator);
  }

  for (const [property, descriptor] of [['contentWindow', contentWindow], ['contentDocument', contentDocument]]) {
    Object.defineProperty(HTMLIFrameElement.prototype, property, {
      ...descriptor,
      get() {
        prepare(this);
        return descriptor.get.call(this);
      },
    });
  }
}
