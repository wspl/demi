// WebSockets of a preview document. A service worker never sees them, so the runtime opens
// them through the Demi page, which connects to the Host's engine
// (`docs/browser/preview.md` § The forwarder and the relay). The object behaves as the browser's own.
import { available, on, send } from './document-channel.js';
import { isProxyUrl, logicalUrl } from './url-map.js';

export function installWebSocketRuntime(currentBaseUrl) {
  const Native = globalThis.WebSocket;
  if (!Native || !available()) return;
  const token = /^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/;
  const NativeURL = globalThis.URL;
  const sockets = new Map();
  let nextId = 1;
  const states = { CONNECTING: 0, OPEN: 1, CLOSING: 2, CLOSED: 3 };
  // Event handler properties (onopen and the rest), by socket.
  const handlers = new WeakMap();

  class WebSocket extends EventTarget {
    #id = nextId++;
    #url;
    #protocol = '';
    #extensions = '';
    #state = 0;
    #binaryType = 'blob';
    // Sends in order, also those that wait for a Blob's bytes.
    #queue = Promise.resolve();

    constructor(url, protocols = []) {
      super();
      let address = new NativeURL(String(url), currentBaseUrl());
      // A page may build the address from what the browser shows it (a preview address): the
      // socket goes to the address that stands behind it.
      const web = new NativeURL(address.href);
      if (web.protocol === 'ws:') web.protocol = 'http:';
      if (web.protocol === 'wss:') web.protocol = 'https:';
      if (isProxyUrl(web.href)) address = new NativeURL(logicalUrl(web.href));
      if (address.protocol === 'http:') address.protocol = 'ws:';
      if (address.protocol === 'https:') address.protocol = 'wss:';
      if (!['ws:', 'wss:'].includes(address.protocol) || address.hash) throw new DOMException(`Failed to construct 'WebSocket': The URL '${url}' is invalid.`, 'SyntaxError');
      this.#url = address.href;
      const list = typeof protocols === 'string' ? [protocols] : [...protocols];
      const seen = new Set();
      for (const protocol of list.map(String)) {
        if (!token.test(protocol) || seen.has(protocol.toLowerCase())) throw new DOMException(`Failed to construct 'WebSocket': The subprotocol '${protocol}' is invalid.`, 'SyntaxError');
        seen.add(protocol.toLowerCase());
      }
      sockets.set(this.#id, this);
      send({ type: 'socket-open', id: this.#id, url: this.#url, protocols: typeof protocols === 'string' ? [protocols] : [...protocols] });
    }

    get url() { return this.#url; }
    get readyState() { return this.#state; }
    get protocol() { return this.#protocol; }
    get extensions() { return this.#extensions; }
    get bufferedAmount() { return 0; }
    get binaryType() { return this.#binaryType; }
    set binaryType(value) { if (['blob', 'arraybuffer'].includes(value)) this.#binaryType = value; }

    send(data) {
      if (this.#state === 0) throw new DOMException("Failed to execute 'send' on 'WebSocket': Still in CONNECTING state.", 'InvalidStateError');
      if (this.#state !== 1) return;
      const id = this.#id;
      this.#queue = this.#queue.then(async () => {
        let payload = data;
        if (data instanceof Blob) payload = await data.arrayBuffer();
        else if (ArrayBuffer.isView(data)) payload = data.buffer.slice(data.byteOffset, data.byteOffset + data.byteLength);
        else if (data instanceof ArrayBuffer) payload = data.slice(0);
        else payload = String(data);
        send({ type: 'socket-send', id, data: payload }, payload instanceof ArrayBuffer ? [payload] : []);
      });
    }

    close(code, reason) {
      if (this.#state >= 2) return;
      this.#state = 2;
      send({ type: 'socket-close', id: this.#id, code, reason });
    }

    // From the Demi page.
    receive(message) {
      if (message.type === 'socket-open') {
        this.#state = 1;
        this.#protocol = message.protocol;
        this.#extensions = message.extensions;
        this.dispatchEvent(new Event('open'));
      } else if (message.type === 'socket-message') {
        const data = typeof message.data === 'string' || this.#binaryType === 'arraybuffer' ? message.data : new Blob([message.data]);
        this.dispatchEvent(new MessageEvent('message', { data, origin: new NativeURL(this.#url).origin }));
      } else if (message.type === 'socket-error') {
        this.dispatchEvent(new Event('error'));
      } else if (message.type === 'socket-close') {
        this.#state = 3;
        sockets.delete(this.#id);
        this.dispatchEvent(new CloseEvent('close', { code: message.code, reason: message.reason, wasClean: message.wasClean }));
      }
    }
  }
  for (const name of ['open', 'message', 'error', 'close']) {
    Object.defineProperty(WebSocket.prototype, `on${name}`, {
      configurable: true,
      enumerable: true,
      get() { return handlers.get(this)?.[name] ?? null; },
      set(handler) {
        const own = handlers.get(this) ?? {};
        handlers.set(this, own);
        if (own[name]) this.removeEventListener(name, own[name]);
        own[name] = typeof handler === 'function' ? handler : null;
        if (own[name]) this.addEventListener(name, own[name]);
      },
    });
  }
  for (const [name, value] of Object.entries(states)) {
    Object.defineProperty(WebSocket, name, { value, enumerable: true });
    Object.defineProperty(WebSocket.prototype, name, { value, enumerable: true });
  }
  Object.defineProperty(WebSocket, 'name', { value: 'WebSocket' });
  for (const type of ['socket-open', 'socket-message', 'socket-error', 'socket-close']) {
    on(type, message => sockets.get(message.id)?.receive(message));
  }
  globalThis.WebSocket = WebSocket;
  return Native;
}
