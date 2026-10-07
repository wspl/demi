// New windows of a preview document (`docs/browser/preview.md` § Opening and navigating): window.open, links
// with target="_blank", and target="_top" from a nested frame. A new window opens as a preview
// tab in Demi; the page gets a window object whose messages, navigation and closing go through
// the Demi page, and the new tab gets an opener that works the same way.
import { available, on, send } from './document-channel.js';

// The window objects this realm's runtime made for other tabs: the page sends them messages
// through their own postMessage, which goes through the Demi page.
const remoteWindows = new WeakSet();
export const isRemoteWindow = object => remoteWindows.has(object);

export function installTabRuntime({ currentBaseUrl, currentLogicalUrl, topLevel, bootPath }) {
  if (!available()) return;
  const NativeURL = globalThis.URL;
  const nativeClose = window.close;
  const popups = new Map();
  let nextId = 1;

  const logical = value => new NativeURL(String(value), currentBaseUrl()).href;

  // The parts of a window another origin may use: messages, navigation, closing.
  function remoteWindow({ post, navigate, close, closed }) {
    const location = {};
    for (const name of ['assign', 'replace']) location[name] = value => navigate(logical(value));
    Object.defineProperty(location, 'href', {
      set: value => navigate(logical(value)),
      get: () => { throw new DOMException('Blocked a frame from accessing a cross-origin frame.', 'SecurityError'); },
    });
    const view = {
      postMessage(data, targetOrigin = '/', transfer) {
        const options = targetOrigin !== null && typeof targetOrigin === 'object' ? targetOrigin : { targetOrigin, transfer };
        if (!closed()) post(data, String(options.targetOrigin ?? '/'));
      },
      close,
      focus() {},
      blur() {},
      get closed() { return closed(); },
      get location() { return location; },
      set location(value) { navigate(logical(value)); },
      length: 0,
    };
    for (const name of ['window', 'self', 'frames', 'top', 'parent']) Object.defineProperty(view, name, { get: () => view });
    remoteWindows.add(view);
    return view;
  }

  // The sender is a window object of the runtime's, which MessageEvent's own source does not
  // accept: the event carries it as its own property.
  function message(data, origin, source) {
    const event = new MessageEvent('message', { data, origin });
    Object.defineProperty(event, 'source', { value: source, enumerable: true });
    window.dispatchEvent(event);
  }

  window.open = function open(url = '', target = '_blank') {
    const name = String(target).toLowerCase();
    if (name === '_self' || name === '_parent' || (name === '_top' && topLevel)) {
      if (url !== '') location.href = url;
      return name === '_self' ? window : top;
    }
    if (name === '_top') {
      if (url !== '') send({ type: 'tab-navigate-self', url: logical(url) });
      return null;
    }
    const id = nextId++;
    const state = { closed: false };
    const popup = remoteWindow({
      post: (data, targetOrigin) => send({ type: 'tab-post', id, data, targetOrigin }),
      navigate: address => send({ type: 'tab-navigate', id, url: address }),
      close: () => send({ type: 'tab-close', id }),
      closed: () => state.closed,
    });
    popups.set(id, { popup, state });
    send({ type: 'tab-open', id, url: url === '' ? '' : logical(url) });
    return popup;
  };
  on('tab-closed', ({ id }) => {
    const entry = popups.get(id);
    if (entry) entry.state.closed = true;
  });
  on('tab-message', ({ id, data, origin }) => message(data, origin, popups.get(id)?.popup ?? null));

  // This tab's opener, once the Demi page says there is one.
  let opener = null;
  on('opener', () => {
    opener = remoteWindow({
      post: (data, targetOrigin) => send({ type: 'opener-post', data, targetOrigin }),
      navigate: address => send({ type: 'opener-navigate', url: address }),
      close() {},
      closed: () => false,
    });
  });
  on('opener-message', ({ data, origin }) => message(data, origin, opener));
  Object.defineProperty(window, 'opener', { configurable: true, enumerable: true, get: () => opener, set: value => { opener = value; } });
  // A tab's top document hears of its opener when its channel connects, which every document's
  // does at once (see runtime.mjs).
  window.close = function close() {
    if (topLevel) send({ type: 'close-self' });
    else nativeClose.call(this);
  };

  // Links to a new window or the tab's top, after the page's own handlers had their say.
  window.addEventListener('click', event => {
    const link = event.target?.closest?.('a[href], area[href]');
    if (!link || event.defaultPrevented || event.button !== 0) return;
    const target = (link.getAttribute('target') ?? '').toLowerCase();
    const newWindow = target === '_blank' || event.metaKey || event.ctrlKey || event.shiftKey;
    if (!newWindow && !(target === '_top' && !topLevel)) return;
    event.preventDefault();
    window.open(link.href, newWindow ? '_blank' : '_top');
  });

  if (topLevel) installTabPage({ currentLogicalUrl, bootPath });
}

// What a tab's top document tells the Demi page of the page it shows, for the tab's address bar and
// strip: its address, title and icon; each move of the tab's history, which the tab counts its pages
// to go back and forward to by; that it leaves, as its tab starts loading; and the bar's Back,
// Forward, Reload and Stop, which act in the page.
function installTabPage({ currentLogicalUrl, bootPath }) {
  // The browser's, before a page's polyfill replaces it.
  const NativeURL = globalThis.URL;
  const navigation = globalThis.navigation;
  let reported = '';
  const report = () => {
    const icon = document.querySelector('link[rel~="icon"][href]');
    const page = {
      url: currentLogicalUrl(),
      title: document.title,
      icon: icon ? new NativeURL(icon.getAttribute('href'), document.baseURI).href : new NativeURL('/favicon.ico', currentLogicalUrl()).href,
    };
    const text = JSON.stringify(page);
    if (text === reported) return;
    reported = text;
    send({ type: 'tab-page', page });
  };
  // A move of the tab's history: the entry it shows now, by its key, and how it got there. The
  // Navigation API sees only this origin's entries, and a page's history spans origins: the tab
  // keeps the whole list from these moves.
  const moved = navigationType => send({ type: 'tab-entry', key: navigation.currentEntry?.key ?? '', navigationType });
  if (navigation) {
    const activation = navigation.activation;
    // The boot page reached this document by replacing itself: the move was the boot page's own,
    // a new page, as a link or the address bar opens one.
    const fromBoot = activation?.from && new NativeURL(activation.from.url).pathname === bootPath;
    moved(fromBoot || !activation ? 'push' : activation.navigationType);
    navigation.addEventListener('currententrychange', event => {
      if (event.navigationType) moved(event.navigationType);
    });
  }
  // The title and the icon change with the head; the address with the history.
  const watch = () => new MutationObserver(report).observe(document.head ?? document.documentElement, { subtree: true, childList: true, characterData: true, attributes: true, attributeFilter: ['href', 'rel'] });
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', () => { watch(); report(); }, { once: true });
  else watch();
  report();
  navigation?.addEventListener('currententrychange', report);
  window.addEventListener('popstate', report);
  window.addEventListener('hashchange', report);
  window.addEventListener('load', report);
  window.addEventListener('pagehide', () => send({ type: 'tab-leaving' }));
  // A move with no page that way is no move: its promises' rejection says so, to nobody.
  const quiet = result => { result?.committed?.catch(() => {}); result?.finished?.catch(() => {}); };
  // Back and Forward move this frame's own history where the next page is of this origin; to another
  // origin's, which the Navigation API cannot see, the tab's session history goes there, as a
  // browser's Back does.
  on('tab-command', ({ command }) => {
    if (command === 'back') navigation?.canGoBack ? quiet(navigation.back()) : window.history.back();
    else if (command === 'forward') navigation?.canGoForward ? quiet(navigation.forward()) : window.history.forward();
    else if (command === 'reload') location.reload();
    else if (command === 'stop') window.stop();
  });
}
