// document.cookie and the Cookie Store API for a preview document. The Host's engine keeps the
// cookies (`docs/browser/preview.md` § The preview engine); this realm keeps a view of the ones its scripts
// see. A write changes the view at once and goes to the engine through the forwarder, which
// sends this document's later requests only after it.
export function installCookieRuntime(boot, currentLogicalUrl) {
  const nativeFetch = globalThis.fetch;
  // The browser's, before a page's polyfill replaces URL (Juejin's lacks URL.parse).
  const parseUrl = URL.parse;
  // A srcdoc document reads its parent's cookies.
  const cookieAddress = () => /^https?:/.test(currentLogicalUrl()) ? currentLogicalUrl() : boot.base;
  const endpoint = () => `/__demi/host/cookie?url=${encodeURIComponent(cookieAddress())}`;
  let view = boot.cookie ?? '';
  // The view the engine sends back replaces the local one unless a later write is on its way.
  let writes = 0;

  // A cookie the browser refuses to store from a script (RFC 6265bis § 5.7): Secure from an
  // insecure page, and the name prefixes' requirements.
  function refused(name, lower) {
    const address = parseUrl(cookieAddress());
    const secure = address?.protocol === 'https:' || ['localhost', '127.0.0.1', '[::1]'].includes(address?.hostname);
    const prefix = name.toLowerCase();
    if (lower.includes('secure') && !secure) return true;
    if (prefix.startsWith('__secure-') && !lower.includes('secure')) return true;
    return prefix.startsWith('__host-') && (!lower.includes('secure') || !lower.includes('path=/') || lower.some(attribute => attribute.startsWith('domain=')));
  }

  function apply(text) {
    const [pair, ...attributes] = String(text).split(';');
    const index = pair.indexOf('=');
    const name = (index < 0 ? '' : pair.slice(0, index)).trim();
    const value = (index < 0 ? pair : pair.slice(index + 1)).trim();
    const lower = attributes.map(attribute => attribute.trim().toLowerCase());
    if (refused(name, lower)) return;
    const maxAge = lower.find(attribute => attribute.startsWith('max-age='));
    const expires = attributes.map(attribute => attribute.trim()).find(attribute => attribute.toLowerCase().startsWith('expires='));
    const removed = (maxAge && Number(maxAge.slice(8)) <= 0) || (expires && Date.parse(expires.slice(8)) <= Date.now());
    const pairs = view ? view.split('; ') : [];
    const position = pairs.findIndex(existing => (existing.includes('=') ? existing.slice(0, existing.indexOf('=')) : '') === name);
    const entry = name ? `${name}=${value}` : value;
    if (removed) {
      if (position >= 0) pairs.splice(position, 1);
    } else if (position >= 0) {
      pairs[position] = entry;
    } else {
      pairs.push(entry);
    }
    view = pairs.join('; ');
  }

  // A failed refresh leaves the view as it was: the next change refreshes it again.
  function refresh(request) {
    const written = writes;
    return request.then(response => response.text()).then(text => {
      if (written === writes) view = text;
    }, () => {});
  }

  const cookie = Object.getOwnPropertyDescriptor(Document.prototype, 'cookie');
  Object.defineProperty(Document.prototype, 'cookie', {
    ...cookie,
    get() {
      if (this !== document) return cookie.get.call(this);
      return view;
    },
    set(value) {
      if (this !== document) return cookie.set.call(this, value);
      apply(value);
      writes++;
      refresh(nativeFetch(endpoint(), { method: 'POST', body: String(value), keepalive: true }));
    },
  });

  // The Cookie Store API over the same view. Cookies read from document.cookie carry only
  // names and values. The methods replace the browser's on CookieStore.prototype, so a page
  // that wraps the prototype's methods (Transcend's consent manager) still reaches these.
  const readCookies = () => (view ? view.split('; ') : []).map(pair => {
    const index = pair.indexOf('=');
    return index < 0 ? { name: '', value: pair } : { name: pair.slice(0, index), value: pair.slice(index + 1) };
  });
  const methods = {
    async getAll(options) {
      const name = typeof options === 'string' ? options : options?.name;
      return readCookies().filter(entry => name === undefined || entry.name === name).map(entry => ({ ...entry, path: '/', domain: null, expires: null, secure: true, sameSite: 'strict', partitioned: false }));
    },
    async get(options) {
      return (await methods.getAll(options))[0] ?? null;
    },
    async set(name, value) {
      const options = typeof name === 'object' ? name : { name, value };
      const attributes = [`${options.name}=${options.value}`, `Path=${options.path ?? '/'}`];
      if (options.domain) attributes.push(`Domain=${options.domain}`);
      if (options.expires) attributes.push(`Expires=${new Date(options.expires).toUTCString()}`);
      if (options.sameSite) attributes.push(`SameSite=${options.sameSite}`);
      document.cookie = attributes.join('; ');
    },
    async delete(options) {
      const name = typeof options === 'string' ? options : options.name;
      document.cookie = `${name}=; Path=${options?.path ?? '/'}; Max-Age=0`;
    },
  };
  if (globalThis.CookieStore) {
    for (const [name, value] of Object.entries(methods)) {
      Object.defineProperty(CookieStore.prototype, name, { ...Object.getOwnPropertyDescriptor(CookieStore.prototype, name), value });
    }
  }
  // A response that changed cookies: read the view again.
  return { cookiesChanged: () => refresh(nativeFetch(endpoint())) };
}
