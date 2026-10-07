// The runtime's wrappers look like the browser functions they replace. Page scripts, bot
// protection among them, read a function's source, name and length to tell whether it was
// replaced (`docs/browser/preview.md` § The runtime, Appearance). The runtime records the browser's
// functions before it installs anything and, once it has, gives each replacement the
// original's name, length and source text.

// The objects whose functions the runtime may replace: the global object, its interfaces'
// prototypes and static members, and the navigator, document and history instances.
function targets() {
  const objects = new Set([globalThis, globalThis.navigator, globalThis.document, globalThis.history].filter(Boolean));
  for (const name of Object.getOwnPropertyNames(globalThis)) {
    const value = Reflect.getOwnPropertyDescriptor(globalThis, name)?.value;
    if (typeof value !== 'function') continue;
    objects.add(value);
    if (value.prototype && typeof value.prototype === 'object') objects.add(value.prototype);
  }
  return objects;
}

// The browser's functions, by object and property, before the runtime changes them.
export function recordNatives() {
  const record = new Map();
  for (const object of targets()) {
    const functions = new Map();
    for (const key of Reflect.ownKeys(object)) {
      const descriptor = Reflect.getOwnPropertyDescriptor(object, key);
      const parts = ['value', 'get', 'set'].filter(part => typeof descriptor?.[part] === 'function');
      if (parts.length) functions.set(key, Object.fromEntries(parts.map(part => [part, descriptor[part]])));
    }
    record.set(object, functions);
  }
  return record;
}

// Give every function the runtime put in place of a recorded one the original's looks.
export function disguiseReplacements(record) {
  const originals = new WeakMap();
  const disguise = (replacement, original) => {
    originals.set(replacement, original);
    for (const property of ['name', 'length']) {
      // A function whose name or length cannot change keeps its own; it still reads native.
      try {
        Object.defineProperty(replacement, property, { value: original[property], configurable: true });
      } catch {}
    }
  };
  // The browser's function a property had before, on the object or, for one the runtime
  // set on an instance (history.pushState), on its prototype chain.
  const recorded = (object, key) => {
    for (let owner = object; owner; owner = Object.getPrototypeOf(owner)) {
      const before = record.get(owner)?.get(key);
      if (before) return before;
    }
    return undefined;
  };
  for (const object of record.keys()) {
    for (const key of Reflect.ownKeys(object)) {
      const before = recorded(object, key);
      const after = Reflect.getOwnPropertyDescriptor(object, key);
      for (const [part, original] of Object.entries(before ?? {})) {
        const replacement = after?.[part];
        if (typeof replacement === 'function' && replacement !== original) disguise(replacement, original);
      }
    }
  }
  const nativeToString = Function.prototype.toString;
  const toString = {
    toString() {
      return nativeToString.call(originals.get(this) ?? this);
    },
  }.toString;
  disguise(toString, nativeToString);
  Object.defineProperty(Function.prototype, 'toString', { ...Object.getOwnPropertyDescriptor(Function.prototype, 'toString'), value: toString });
}
