// The package's root also initializes Web IDL wrappers requiring SharedArrayBuffer.
// Use its documented parsing machinery directly in ordinary browser realms.
import { parseURL, basicURLParse, serializeURL } from 'whatwg-url/lib/url-state-machine.js';
import { logicalUrl, mapUrl } from './url-map.js';

const nativeOwnKeys = Reflect.ownKeys;

// Computed properties perform the language's ToPropertyKey conversion exactly once.
// There is no standard callable ToPropertyKey API; this preserves Symbol keys too.
export function propertyKey(property) {
  return nativeOwnKeys({ [property]: 0 })[0];
}

// Adapt Location operations and reflection while retaining native receiver checks.
export function installLocationRuntime(realLocation, currentBaseUrl, ancestors) {
  // The browser's URL, not a page's replacement.
  const NativeURL = globalThis.URL;
  const native = {
    descriptor: Object.getOwnPropertyDescriptor,
    descriptors: Object.getOwnPropertyDescriptors,
    define: Object.defineProperty,
    defineAll: Object.defineProperties,
    prototype: Object.getPrototypeOf,
    apply: Reflect.apply,
    get: Reflect.get,
    set: Reflect.set,
    reflectDescriptor: Reflect.getOwnPropertyDescriptor,
    ownKeys: Reflect.ownKeys,
  };
  const hrefDescriptor = native.descriptor(realLocation, 'href');
  const windowLocation = native.descriptor(globalThis, 'location');
  const documentLocation = native.descriptor(Document.prototype, 'location');
  const ancestorList = Object.freeze(Object.assign(Object.create(null), ancestors, {
    length: ancestors.length,
    item: index => ancestors[index] ?? null,
    contains: origin => ancestors.includes(String(origin)),
    [Symbol.iterator]: () => ancestors[Symbol.iterator](),
  }));
  const views = new WeakMap();
  const originals = new WeakMap();
  const functions = new WeakMap();
  const urlProperties = new Set(['href', 'origin', 'protocol', 'host', 'hostname', 'port', 'pathname', 'search', 'hash']);
  const locationProperties = new Set(Reflect.ownKeys(native.descriptors(realLocation)));

  // Native Location getters brand-check without invoking application properties.
  function isLocation(value) {
    if (!value || (typeof value !== 'object' && typeof value !== 'function')) return false;
    if (originals.has(value) || views.has(value)) return true;
    try {
      native.apply(hrefDescriptor.get, value, []);
      return true;
    } catch (error) {
      if (error.name === 'SecurityError') return true;
      if (error instanceof TypeError) return false;
      throw error;
    }
  }

  // Identify a Window or Document through its native location accessor.
  function ownerLocation(value) {
    if (!value || (typeof value !== 'object' && typeof value !== 'function')) return undefined;
    for (const descriptor of [windowLocation, documentLocation]) {
      if (!descriptor?.get) continue;
      try { return { value: native.apply(descriptor.get, value, []), descriptor }; }
      catch (error) {
        if (error instanceof TypeError) continue;
        throw error;
      }
    }
    return undefined;
  }

  // Keep reflected native function identities stable within this execution realm.
  function wrapFunction(fn, property, kind) {
    if (!fn) return fn;
    let cache = functions.get(fn);
    if (!cache) {
      cache = new Map();
      functions.set(fn, cache);
    }
    const key = `${kind}:${String(property)}`;
    if (cache.has(key)) return cache.get(key);
    const wrapped = new Proxy(fn, {
      apply(target, receiver, args) {
        const original = originals.get(receiver) ?? receiver;
        if (property === 'location') {
          if (kind === 'get') return view(native.apply(target, original, args));
          if (ownerLocation(original) === undefined) return native.apply(target, original, args);
          return native.apply(target, original, [navigationUrl(args[0])]);
        }
        if (!isLocation(original)) return native.apply(target, original, args);
        if (kind === 'get') {
          const result = native.apply(target, original, args);
          // The real list ends with Demi's page and names preview origins; the page sees its
          // logical ancestors, none for the logical top.
          if (property === 'ancestorOrigins') return ancestorList;
          if (!urlProperties.has(property)) return result;
          const url = logicalUrl(native.apply(hrefDescriptor.get, original, []));
          return new NativeURL(url)[property];
        }
        if (kind === 'set') {
          if (property === 'href') return native.apply(target, original, [navigationUrl(args[0])]);
          if (['pathname', 'search', 'hash'].includes(property)) return native.apply(target, original, args);
          const logical = logicalUrl(native.apply(hrefDescriptor.get, original, []));
          if (property === 'protocol') {
            const url = parseURL(logical);
            const result = basicURLParse(`${args[0]}:`, { url, stateOverride: 'scheme start' });
            if (result === null) throw new DOMException('Invalid protocol', 'SyntaxError');
            if (!['http', 'https'].includes(url.scheme)) return;
            return native.apply(hrefDescriptor.set, original, [mapUrl(serializeURL(url), currentBaseUrl(), 'navigation')]);
          }
          const url = new NativeURL(logical);
          url[property] = args[0];
          return native.apply(hrefDescriptor.set, original, [mapUrl(url, currentBaseUrl(), 'navigation')]);
        }
        if (property === 'assign' || property === 'replace') {
          if (!args.length) return native.apply(target, original, args);
          if (property === 'assign') native.apply(hrefDescriptor.get, original, []);
          return native.apply(target, original, [navigationUrl(args[0])]);
        }
        const result = native.apply(target, original, args);
        return property === 'toString' ? logicalUrl(result) : result;
      },
    });
    cache.set(key, wrapped);
    functions.set(wrapped, cache);
    return wrapped;
  }

  // Reflect the same descriptor shape, changing only proxy-sensitive operations.
  function adaptDescriptor(object, property, descriptor) {
    if (!descriptor) return descriptor;
    const original = originals.get(object) ?? object;
    const owner = property === 'location' && ownerLocation(original) !== undefined;
    if (!owner && !isLocation(original)) return descriptor;
    const result = { ...descriptor };
    if (result.get) result.get = wrapFunction(result.get, property, 'get');
    if (result.set) result.set = wrapFunction(result.set, property, 'set');
    if (typeof result.value === 'function') result.value = wrapFunction(result.value, property, 'method');
    return result;
  }

  // Represent a Location with its native prototype and native descriptor flags.
  function view(original) {
    if (!original || originals.has(original)) return original;
    if (views.has(original)) return views.get(original);
    const descriptors = native.descriptors(original);
    const target = Object.create(native.prototype(original));
    for (const key of Reflect.ownKeys(descriptors)) descriptors[key] = adaptDescriptor(original, key, descriptors[key]);
    native.defineAll(target, descriptors);
    const result = new Proxy(target, {
      get(object, key, receiver) {
        native.get(original, key, original);
        return native.get(object, key, receiver);
      },
      set(object, key, value, receiver) {
        return native.set(object, key, value, receiver);
      },
    });
    views.set(original, result);
    originals.set(result, original);
    return result;
  }

  // Route member operations on real Locations and leave ordinary objects untouched.
  function propertyOwner(object, property) {
    if (property === 'location') {
      const original = ownerLocation(object);
      if (original === undefined) return object;
      const owner = {};
      native.define(owner, 'location', {
        get() { return view(original.value); },
        set(value) { native.apply(original.descriptor.set, object, [navigationUrl(value)]); },
      });
      return owner;
    }
    return locationProperties.has(property) && isLocation(object) ? view(originals.get(object) ?? object) : object;
  }

  // Location's invalid-URL exception differs from the URL constructor's TypeError.
  function navigationUrl(value) {
    const text = `${value}`;
    if (!URL.canParse(text, currentBaseUrl())) throw new DOMException('Invalid URL', 'SyntaxError');
    return mapUrl(text, currentBaseUrl(), 'navigation');
  }

  for (const [owner, original] of [[Object, native.descriptor], [Reflect, native.reflectDescriptor]]) {
    owner.getOwnPropertyDescriptor = function(object, property) {
      const target = originals.get(object) ?? object;
      if (target == null || (owner === Reflect && typeof target !== 'object' && typeof target !== 'function')) {
        return original(target, property);
      }
      const key = propertyKey(property);
      return adaptDescriptor(target, key, original(target, key));
    };
  }
  Object.getOwnPropertyDescriptors = function(object) {
    const original = originals.get(object) ?? object;
    const descriptors = native.descriptors(original);
    for (const property of Reflect.ownKeys(descriptors)) {
      descriptors[property] = adaptDescriptor(original, property, descriptors[property]);
    }
    return descriptors;
  };
  for (const [name, kind] of [['__lookupGetter__', 'get'], ['__lookupSetter__', 'set']]) {
    const original = Object.prototype[name];
    Object.prototype[name] = function(property) {
      const receiver = originals.get(this) ?? this;
      if (receiver == null) return native.apply(original, receiver, [property]);
      const key = propertyKey(property);
      const fn = native.apply(original, receiver, [key]);
      const descriptor = adaptDescriptor(receiver, key, { [kind]: fn });
      return descriptor[kind];
    };
  }
  for (const [name, operation] of [['get', native.get], ['set', native.set]]) {
    Reflect[name] = function(...args) {
      const [object, property] = args;
      if (object === null || (typeof object !== 'object' && typeof object !== 'function')) {
        return native.apply(operation, Reflect, args);
      }
      const key = propertyKey(property);
      const receiverIndex = name === 'get' ? 2 : 3;
      const receiver = args.length > receiverIndex ? args[receiverIndex] : object;
      const owner = key === 'location' ? ownerLocation(object) : undefined;
      if (owner) {
        const fn = wrapFunction(owner.descriptor[name], key, name);
        if (name === 'get') return native.apply(fn, receiver, []);
        native.apply(fn, receiver, [args[2]]);
        return true;
      }
      const target = propertyOwner(object, key);
      const mappedReceiver = receiver === object ? target : receiver;
      return name === 'get' ? native.get(target, key, mappedReceiver) : native.set(target, key, args[2], mappedReceiver);
    };
  }
  return { view: view(realLocation), propertyOwner };
}
