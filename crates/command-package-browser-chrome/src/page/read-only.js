((value, maxBytes) => {
  // CDP's JSON mode silently coerces some values. Validate and copy inside
  // Chrome's side-effect check, including getters, before serializing.
  // `undefined` is treated as JSON.stringify treats it: null in an array,
  // left out of an object, and no value as the whole result. Any other value
  // JSON cannot hold fails, naming its place, such as `result[2]`.
  const ancestors = new Set();
  let remaining = maxBytes;
  const fail = (text) => {
    throw new Error(`demi-unsupported-result: ${text}`);
  };
  const isIdentifier = (key) => {
    if (key.length === 0) {
      return false;
    }
    for (let index = 0; index < key.length; index++) {
      const code = key.charCodeAt(index);
      const letter = (code >= 65 && code <= 90) || (code >= 97 && code <= 122) ||
          code === 36 || code === 95;
      const digit = code >= 48 && code <= 57;
      if (!letter && !(digit && index > 0)) {
        return false;
      }
    }
    return true;
  };
  const copy = (value, place, depth) => {
    if (--remaining < 0 || depth > 64) {
      fail('the result exceeds its size or depth limit');
    }
    if (value === null || typeof value === 'boolean') {
      return value;
    }
    if (value === undefined) {
      return undefined;
    }
    if (typeof value === 'number') {
      if (!Number.isFinite(value)) {
        fail(`${place} is ${value}, which JSON cannot hold`);
      }
      return value;
    }
    if (typeof value === 'string') {
      remaining -= value.length;
      if (remaining < 0) {
        fail('the result exceeds its size limit');
      }
      return value;
    }
    if (typeof value === 'function') {
      fail(`${place} is a function`);
    }
    if (typeof value !== 'object') {
      fail(`${place} is a ${typeof value}`);
    }
    if (value instanceof Promise) {
      fail(`${place} is a Promise; eval returns values that are already there and cannot wait`);
    }
    if (ancestors.has(value)) {
      fail(`${place} is a cycle back to an object that holds it`);
    }
    const array = Array.isArray(value);
    const prototype = Object.getPrototypeOf(value);
    if (!array && prototype !== Object.prototype && prototype !== null) {
      if (typeof Node === 'function' && value instanceof Node) {
        fail(`${place} is a DOM node (${value.nodeName}); return its properties instead`);
      }
      const name = prototype.constructor && prototype.constructor.name;
      fail(`${place} is ${name ? `a ${name}` : 'an object of its own kind'}, which JSON cannot hold`);
    }
    ancestors.add(value);
    let result;
    if (array) {
      result = [];
      for (let index = 0; index < value.length; index++) {
        const item = copy(value[index], `${place}[${index}]`, depth + 1);
        result.push(item === undefined ? null : item);
      }
    } else {
      result = Object.create(null);
      for (const key of Object.keys(value)) {
        remaining -= key.length;
        const inner = isIdentifier(key) ? `${place}.${key}` : `${place}[${JSON.stringify(key)}]`;
        const item = copy(value[key], inner, depth + 1);
        if (item !== undefined) {
          result[key] = item;
        }
      }
    }
    ancestors.delete(value);
    return result;
  };
  const result = copy(value, 'result', 0);
  const json = JSON.stringify(result === undefined ? {} : {value: result});
  if (json.length > maxBytes) {
    fail('the result exceeds its size limit');
  }
  return json;
})
