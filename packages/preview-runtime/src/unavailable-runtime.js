// Notifications: Chrome denies a cross-origin frame without asking, and every preview is
// one. A site that was never asked reads "default" (and "prompt" from the Permissions API), and denial without a prompt
// is what bot protection looks for in headless browsers. The page reads that it was never
// asked; a request reads as a prompt the user dismissed, since none can be shown.
function installNotificationPermission() {
  if (!globalThis.Notification) return;
  Object.defineProperty(Notification, 'permission', { ...Object.getOwnPropertyDescriptor(Notification, 'permission'), get: function permission() { return 'default'; } });
  Object.defineProperty(Notification, 'requestPermission', {
    ...Object.getOwnPropertyDescriptor(Notification, 'requestPermission'),
    value: function requestPermission(callback) {
      const answer = Promise.resolve('default');
      if (typeof callback === 'function') answer.then(callback);
      return answer;
    },
  });
  if (!globalThis.Permissions || !globalThis.PermissionStatus) return;
  const notifications = new WeakSet();
  const nativeQuery = Permissions.prototype.query;
  Object.defineProperty(Permissions.prototype, 'query', {
    ...Object.getOwnPropertyDescriptor(Permissions.prototype, 'query'),
    value: function query(descriptor) {
      return nativeQuery.call(this, descriptor).then(status => {
        if (['notifications', 'push'].includes(descriptor?.name)) notifications.add(status);
        return status;
      });
    },
  });
  const nativeState = Object.getOwnPropertyDescriptor(PermissionStatus.prototype, 'state');
  Object.defineProperty(PermissionStatus.prototype, 'state', { ...nativeState, get: function state() { return notifications.has(this) ? 'prompt' : nativeState.get.call(this); } });
}

// The site's own service worker would replace the preview's forwarder
// (`docs/browser/preview.md` § Storage and browser features): the page sees a browser that never lets it
// register one, with the objects in place and every operation declined.
export function installUnavailableApis() {
  installNotificationPermission();
  const decline = name => () => Promise.reject(new DOMException(`${name} is unavailable in a Demi preview`, 'SecurityError'));
  if (!globalThis.Navigator || !Object.getOwnPropertyDescriptor(Navigator.prototype, 'serviceWorker')) return;
  // Never controlled, never ready: what a page sees in a browser without Service Workers.
  const container = Object.assign(new EventTarget(), {
    controller: null,
    ready: new Promise(() => {}),
    oncontrollerchange: null,
    onmessage: null,
    onmessageerror: null,
    register: decline('ServiceWorker'),
    getRegistration: () => Promise.resolve(undefined),
    getRegistrations: () => Promise.resolve([]),
    startMessages() {},
  });
  Object.defineProperty(Navigator.prototype, 'serviceWorker', { configurable: true, enumerable: true, get: () => container });
}
