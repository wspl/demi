((value, maxBytes) => {
  // CDP's JSON mode silently coerces some values. Validate and copy inside
  // Chrome's side-effect check, including getters, before serializing.
  const ancestors = new Set();
  let remaining = maxBytes;
  const copy = (value, depth) => {
    if (--remaining < 0 || depth > 64) {
      throw new Error('Browser result exceeds its size or depth limit');
    }
    if (value === null || typeof value === 'boolean') {
      return value;
    }
    if (typeof value === 'number' && Number.isFinite(value)) {
      return value;
    }
    if (typeof value === 'string') {
      remaining -= value.length;
      if (remaining < 0) {
        throw new Error('Browser result exceeds its size limit');
      }
      return value;
    }
    if (typeof value !== 'object') {
      throw new Error('Unsupported browser result');
    }
    if (ancestors.has(value)) {
      throw new Error('Cyclic browser result');
    }
    const array = Array.isArray(value);
    if (!array && Object.getPrototypeOf(value) !== Object.prototype &&
        Object.getPrototypeOf(value) !== null) {
      throw new Error('Unsupported browser result object');
    }
    ancestors.add(value);
    const result = array ? [] : Object.create(null);
    for (const key of Reflect.ownKeys(value)) {
      if (array && key === 'length') {
        continue;
      }
      if (typeof key !== 'string') {
        throw new Error('Symbol key in browser result');
      }
      if (!Object.prototype.propertyIsEnumerable.call(value, key)) {
        continue;
      }
      if (array && key !== String(result.length)) {
        throw new Error('Unsupported array property in browser result');
      }
      remaining -= key.length;
      result[key] = copy(value[key], depth + 1);
    }
    if (array && Object.keys(result).length !== value.length) {
      throw new Error('Sparse array in browser result');
    }
    ancestors.delete(value);
    return result;
  };
  const json = JSON.stringify(copy(value, 0));
  if (json.length > maxBytes) {
    throw new Error('Browser result exceeds its size limit');
  }
  return json;
})
