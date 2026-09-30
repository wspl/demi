function(kind, expected, exact) {
  const pattern = kind === 'text-pattern' ? new RegExp(expected) : null;
  if (kind === 'css') document.querySelector(expected);
  const match = text => pattern ? pattern.test(text) : exact ? text === expected : text.includes(expected);
  const rendered = new WeakMap();
  // DOM text locators include hidden elements. innerText supplies rendered
  // text where available and the browser's text fallback on hidden elements.
  const visibleText = element => {
    if (!rendered.has(element)) {
      rendered.set(element, (element.innerText || element.textContent || '').trim());
    }
    return rendered.get(element);
  };
  const matches = element => {
    if (kind === 'css') return element.matches(expected);
    if (kind === 'placeholder') return element.getAttribute('placeholder') === expected;
    if (kind === 'test-id') return element.getAttribute('data-testid') === expected;
    if (kind === 'label') {
      const labelledBy = element.getAttribute('aria-labelledby');
      if (labelledBy) {
        const root = element.getRootNode();
        const labels = labelledBy.trim().split(/\s+/).map(id => root.getElementById(id)).filter(Boolean);
        if (labels.length) return match(labels.map(label => label.textContent || '').join(' ').trim());
      }
      return Array.from(element.labels || []).some(label => match((label.textContent || '').trim()));
    }
    if (['script', 'style', 'noscript'].includes(element.localName)) return false;
    const text = visibleText(element);
    if (!text || !match(text)) return false;
    // Prefer the smallest rendered text owner instead of every matching ancestor.
    return !Array.from(element.children).some(child => match(visibleText(child)));
  };
  const result = [];
  const pending = Array.from(document.children).reverse();
  while (pending.length) {
    const element = pending.pop();
    if (matches(element)) result.push(element);
    // Frames are scanned separately in their captured execution contexts.
    const children = Array.from(element.children);
    if (element.shadowRoot) children.push(...element.shadowRoot.children);
    pending.push(...children.reverse());
  }
  return result;
}
