// The document's own channel to the Demi page (the preview domain service's client script gives it): the labels the
// runtime maps, and WebSockets, which no service worker sees. A worker has none.
let port;
const handlers = new Map();

function channel() {
  port ??= globalThis.__demiPreview?.channel().then(opened => {
    opened.onmessage = event => handlers.get(event.data.type)?.(event.data);
    return opened;
  });
  return port;
}

// Messages leave in the order they are sent, once the channel is open.
export function send(message, transfer = []) {
  channel()?.then(opened => opened.postMessage(message, transfer));
}

export function on(type, handler) {
  handlers.set(type, handler);
}

export function available() {
  return Boolean(globalThis.__demiPreview);
}
