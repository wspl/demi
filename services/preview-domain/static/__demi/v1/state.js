// The page state's codec (docs/browser/preview.md § Page state): one origin's localStorage,
// sessionStorage and IndexedDB, read from a page and written into another. Both browsers run this
// one file: the preview's state frame (`state.html`) in the user's browser, and demi-browser, in a
// world of its own, in the agent's browser over CDP. It is one expression, so CDP can evaluate it.
//
// IndexedDB values keep their structured-clone types in a tagged encoding: a string, a boolean,
// null or a finite number other than -0 stands for itself; anything else is an object whose `$`
// names its type. A value it cannot encode, such as a CryptoKey or a reference the value shares
// with itself, is left out, and its store named in `skipped`.
globalThis.__demiPageState ??= (() => {
  // How long writing waits for the other pages of the origin to close a database it replaces.
  const BLOCKED_MS = 3000;
  const TYPED = ['Int8Array', 'Uint8Array', 'Uint8ClampedArray', 'Int16Array', 'Uint16Array', 'Int32Array',
    'Uint32Array', 'Float16Array', 'Float32Array', 'Float64Array', 'BigInt64Array', 'BigUint64Array']
    .filter(name => typeof globalThis[name] === 'function');

  class Unencodable extends Error {}

  const base64 = bytes => {
    let text = '';
    for (let at = 0; at < bytes.length; at += 0x8000) text += String.fromCharCode(...bytes.subarray(at, at + 0x8000));
    return btoa(text);
  };
  const bytesOf = text => Uint8Array.from(atob(text), character => character.charCodeAt(0));
  const viewBytes = view => new Uint8Array(view.buffer, view.byteOffset, view.byteLength).slice();

  // A value in the tagged encoding. `ancestors` holds the objects being encoded around it.
  async function encode(value, ancestors = new Set()) {
    if (value === null || typeof value === 'string' || typeof value === 'boolean') return value;
    if (typeof value === 'number') return Number.isFinite(value) && !Object.is(value, -0) ? value : { $: 'number', v: String(value) };
    if (value === undefined) return { $: 'undefined' };
    if (typeof value === 'bigint') return { $: 'bigint', v: value.toString() };
    if (typeof value !== 'object' || ancestors.has(value)) throw new Unencodable();
    ancestors.add(value);
    try {
      const tag = Object.prototype.toString.call(value).slice(8, -1);
      const all = async values => Promise.all(values.map(each => encode(each, ancestors)));
      if (tag === 'Date') return { $: 'date', v: value.getTime() };
      if (tag === 'RegExp') return { $: 'regexp', source: value.source, flags: value.flags };
      if (tag === 'ArrayBuffer') return { $: 'arraybuffer', v: base64(new Uint8Array(value)) };
      if (tag === 'DataView') return { $: 'dataview', v: base64(viewBytes(value)) };
      if (TYPED.includes(tag)) return { $: 'typed', type: tag, v: base64(viewBytes(value)) };
      if (tag === 'Blob' || tag === 'File') {
        const blob = { $: 'blob', type: value.type, v: base64(new Uint8Array(await value.arrayBuffer())) };
        return tag === 'File' ? { ...blob, $: 'file', name: value.name, lastModified: value.lastModified } : blob;
      }
      if (tag === 'Map') return { $: 'map', v: await Promise.all([...value].map(entry => all(entry))) };
      if (tag === 'Set') return { $: 'set', v: await all([...value]) };
      if (tag === 'Boolean' || tag === 'Number' || tag === 'String') return { $: 'boxed', v: await encode(value.valueOf(), ancestors) };
      if (tag === 'Error') return { $: 'error', name: value.name, message: value.message };
      if (Array.isArray(value)) return { $: 'array', v: await all(Array.from(value)) };
      const prototype = Object.getPrototypeOf(value);
      if (tag === 'Object' && (prototype === Object.prototype || prototype === null)) {
        const entries = await Promise.all(Object.keys(value).map(async key => [key, await encode(value[key], ancestors)]));
        return { $: 'object', v: Object.fromEntries(entries) };
      }
      throw new Unencodable();
    } finally {
      ancestors.delete(value);
    }
  }

  function decode(value) {
    if (value === null || typeof value !== 'object') return value;
    switch (value.$) {
      case 'number': return Number(value.v);
      case 'undefined': return undefined;
      case 'bigint': return BigInt(value.v);
      case 'date': return new Date(value.v);
      case 'regexp': return new RegExp(value.source, value.flags);
      case 'arraybuffer': return bytesOf(value.v).buffer;
      case 'dataview': return new DataView(bytesOf(value.v).buffer);
      case 'typed': return new globalThis[value.type](bytesOf(value.v).buffer);
      case 'blob': return new Blob([bytesOf(value.v)], { type: value.type });
      case 'file': return new File([bytesOf(value.v)], value.name, { type: value.type, lastModified: value.lastModified });
      case 'map': return new Map(value.v.map(([key, entry]) => [decode(key), decode(entry)]));
      case 'set': return new Set(value.v.map(decode));
      case 'boxed': return Object(decode(value.v));
      case 'error': return Object.assign(new Error(value.message), { name: value.name });
      case 'array': return value.v.map(decode);
      case 'object': return Object.fromEntries(Object.entries(value.v).map(([key, entry]) => [key, decode(entry)]));
      default: throw new Error(`the page state has a value of an unknown type: ${value.$}`);
    }
  }

  const done = request => new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
  const items = storage => Array.from({ length: storage.length }, (_, index) => {
    const key = storage.key(index);
    return { key, value: storage.getItem(key) };
  });

  // The storage of this document's origin, as `origin` names it to the other browser.
  async function dump(origin) {
    const databases = [];
    const skipped = [];
    for (const { name, version } of await indexedDB.databases()) {
      const database = await done(indexedDB.open(name));
      try {
        const stores = [];
        for (const storeName of database.objectStoreNames) {
          const store = database.transaction(storeName, 'readonly').objectStore(storeName);
          const [keys, values] = await Promise.all([done(store.getAllKeys()), done(store.getAll())]);
          const records = [];
          for (const [index, key] of keys.entries()) {
            try {
              records.push([await encode(key), await encode(values[index])]);
            } catch (error) {
              if (!(error instanceof Unencodable)) throw error;
              const where = `${name}/${storeName}`;
              if (!skipped.includes(where)) skipped.push(where);
            }
          }
          stores.push({
            name: storeName,
            keyPath: store.keyPath,
            autoIncrement: store.autoIncrement,
            indexes: Array.from(store.indexNames, indexName => {
              const index = store.index(indexName);
              return { name: indexName, keyPath: index.keyPath, unique: index.unique, multiEntry: index.multiEntry };
            }),
            records,
          });
        }
        databases.push({ name, version, stores });
      } finally {
        database.close();
      }
    }
    return { origin, local: items(localStorage), session: items(sessionStorage), databases, skipped };
  }

  // Deletes the database `name`; a page of the origin that keeps it open past `BLOCKED_MS` keeps it.
  const deleted = name => new Promise((resolve, reject) => {
    const request = indexedDB.deleteDatabase(name);
    let timer;
    request.onsuccess = () => { clearTimeout(timer); resolve(true); };
    request.onerror = () => { clearTimeout(timer); reject(request.error); };
    request.onblocked = () => { timer = setTimeout(() => resolve(false), BLOCKED_MS); };
  });

  // Replaces this origin's storage with `state`'s; answers the databases another page kept open.
  async function seed(state) {
    localStorage.clear();
    for (const { key, value } of state.local) localStorage.setItem(key, value);
    sessionStorage.clear();
    for (const { key, value } of state.session) sessionStorage.setItem(key, value);
    const kept = [];
    for (const { name } of await indexedDB.databases()) {
      if (!(await deleted(name))) kept.push(name);
    }
    for (const description of state.databases) {
      if (kept.includes(description.name)) continue;
      const opening = indexedDB.open(description.name, description.version);
      opening.onupgradeneeded = () => {
        for (const store of description.stores) {
          const created = opening.result.createObjectStore(store.name, { keyPath: store.keyPath ?? undefined, autoIncrement: store.autoIncrement });
          for (const index of store.indexes) created.createIndex(index.name, index.keyPath, { unique: index.unique, multiEntry: index.multiEntry });
        }
      };
      const database = await done(opening);
      try {
        const names = description.stores.map(store => store.name);
        if (names.length === 0) continue;
        const transaction = database.transaction(names, 'readwrite');
        for (const store of description.stores) {
          const target = transaction.objectStore(store.name);
          for (const [key, value] of store.records) {
            if (store.keyPath === null) target.put(decode(value), decode(key));
            else target.put(decode(value));
          }
        }
        await new Promise((resolve, reject) => {
          transaction.oncomplete = resolve;
          transaction.onerror = () => reject(transaction.error);
          transaction.onabort = () => reject(transaction.error);
        });
      } finally {
        database.close();
      }
    }
    return { kept };
  }

  return { dump, seed };
})();
