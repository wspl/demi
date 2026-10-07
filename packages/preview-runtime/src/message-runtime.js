// Window messages between preview documents. A window's real origin is its preview origin,
// which a page does not know: it names logical origins. The runtime sends with "*" inside an
// envelope carrying the logical origins; the receiving runtime checks the sender's claim
// against the real sender origin's label, enforces the target, and shows the page the logical
// sender (`docs/browser/preview.md` § The runtime).
import { currentRewriter } from './url-map.js';
import { on, send } from './document-channel.js';
import { isRemoteWindow } from './tab-runtime.js';

const marker = '__proxyMessage';

export function installMessageRuntime(currentOrigin) {
  // The browser's URL parser, not a page's replacement.
  const parseUrl = URL.parse;
  const nativeData = Object.getOwnPropertyDescriptor(MessageEvent.prototype, 'data').get;
  const nativeOrigin = Object.getOwnPropertyDescriptor(MessageEvent.prototype, 'origin').get;
  const envelopeOf = event => {
    const data = nativeData.call(event);
    return data !== null && typeof data === 'object' && data[marker] === 1 ? data : undefined;
  };
  // The logical origin behind a preview origin this realm knows; undefined for an unknown one.
  const logicalOrigin = origin => {
    const parsed = currentRewriter().parsePreviewUrl(`${origin}/`);
    if (!parsed) return origin;
    return JSON.parse(currentRewriter().environmentOf(JSON.parse(parsed).label) ?? 'null')?.origin;
  };
  for (const [property, read] of [['data', envelope => envelope.data], ['origin', envelope => envelope.origin]]) {
    const descriptor = Object.getOwnPropertyDescriptor(MessageEvent.prototype, property);
    Object.defineProperty(MessageEvent.prototype, property, {
      ...descriptor,
      get() {
        const envelope = envelopeOf(this);
        if (envelope) return read(envelope);
        const value = (property === 'data' ? nativeData : nativeOrigin).call(this);
        return property === 'origin' ? logicalOrigin(value) ?? value : value;
      },
    });
  }
  if (!globalThis.window) return { owner: object => object };

  // Labels the Demi page tells this document about, for senders it has not met.
  const lookups = new Map();
  on('label', message => {
    if (message.environment) currentRewriter().learn(JSON.stringify({ [message.label]: message.environment }));
    lookups.get(message.label)?.forEach(resolve => resolve());
    lookups.delete(message.label);
  });
  const lookUp = label => new Promise(resolve => {
    lookups.set(label, [...(lookups.get(label) ?? []), resolve]);
    send({ type: 'label', label });
  });
  // Messages wait, in order, while an unknown sender is looked up.
  let delivery = Promise.resolve();
  const redelivered = new WeakSet();

  function accepted(event, envelope) {
    const sender = logicalOrigin(nativeOrigin.call(event));
    return sender === envelope.origin && (envelope.target === '*' || envelope.target === currentOrigin());
  }

  // Registered before any page script, so it runs before the page's own listeners.
  window.addEventListener('message', event => {
    const envelope = envelopeOf(event);
    if (!envelope || redelivered.has(event)) return;
    const parsed = currentRewriter().parsePreviewUrl(`${nativeOrigin.call(event)}/`);
    const label = parsed && JSON.parse(parsed).label;
    if (label && currentRewriter().environmentOf(label) === undefined) {
      event.stopImmediatePropagation();
      const copy = new MessageEvent('message', { data: envelope, origin: nativeOrigin.call(event), source: event.source, ports: [...event.ports] });
      delivery = delivery.then(() => lookUp(label)).then(() => {
        if (!accepted(copy, envelope)) return;
        redelivered.add(copy);
        window.dispatchEvent(copy);
      });
      return;
    }
    if (!accepted(event, envelope)) event.stopImmediatePropagation();
  }, true);

  function targetOf(value) {
    const text = String(value);
    if (text === '*') return '*';
    if (text === '/') return currentOrigin();
    const url = parseUrl(text);
    if (!url) throw new DOMException(`Invalid target origin '${text}' in a call to 'postMessage'.`, 'SyntaxError');
    return url.origin;
  }
  const nativePostMessage = window.postMessage;
  function post(target, message, targetOrigin, transfer) {
    const options = targetOrigin !== null && typeof targetOrigin === 'object' ? targetOrigin : { targetOrigin, transfer };
    const envelope = { [marker]: 1, origin: currentOrigin(), target: targetOf(options.targetOrigin ?? '/'), data: message };
    // This realm's native postMessage, whatever the target's realm: the browser names the
    // calling realm's window as the source. A same-origin window's own postMessage is its
    // runtime's, which would wrap the envelope again.
    const sendTo = isRemoteWindow(target) ? target.postMessage : nativePostMessage;
    return sendTo.call(target, envelope, { targetOrigin: '*', transfer: options.transfer ?? [] });
  }
  window.postMessage = (message, targetOrigin, transfer) => post(window, message, targetOrigin, transfer);
  return {
    owner(object) {
      if (!object || object.window !== object) return object;
      return { postMessage: (message, targetOrigin, transfer) => post(object, message, targetOrigin, transfer) };
    },
  };
}
