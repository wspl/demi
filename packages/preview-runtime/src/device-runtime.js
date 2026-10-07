// A tab of the user's browser in Mobile (docs/browser/preview.md § Mobile): the page's scripts see the
// phone its requests describe. The engine names it in the document's boot data, `device` (never
// `client`, which names the client script); the runtime gives Navigator's userAgent, appVersion,
// platform and maxTouchPoints, and NavigatorUAData's mobile and platform, before the page's scripts
// run. A worker's navigator, touch events, the pointer and hover media features and the screen's size
// stay the user's own.

// What Chrome on an Android phone says of itself.
const PLATFORM = 'Linux armv81';
const TOUCH_POINTS = 5;

// Gives `prototype`'s getter `name` the value `value`, still checking its receiver as the browser's does.
function answer(prototype, name, value) {
  const descriptor = prototype && Object.getOwnPropertyDescriptor(prototype, name);
  if (!descriptor?.get) return;
  const original = descriptor.get;
  Object.defineProperty(prototype, name, {
    ...descriptor,
    get() {
      original.call(this);
      return value;
    },
  });
}

export function installDevice(device) {
  if (!device?.mobile) return;
  const navigatorPrototype = globalThis.Navigator?.prototype;
  answer(navigatorPrototype, 'userAgent', device.userAgent);
  answer(navigatorPrototype, 'appVersion', device.userAgent.replace(/^Mozilla\//, ''));
  answer(navigatorPrototype, 'platform', PLATFORM);
  answer(navigatorPrototype, 'maxTouchPoints', TOUCH_POINTS);
  const hints = globalThis.NavigatorUAData?.prototype;
  answer(hints, 'mobile', true);
  answer(hints, 'platform', device.platform);
}
