function(kind, expected, exact) {
  const match = text => exact ? text === expected : text.includes(expected);
  const rendered = new WeakMap();
  // innerText falls back to raw text on hidden elements; exclude them explicitly.
  const visibleText = element => {
    if (!rendered.has(element)) {
      const style = element.ownerDocument.defaultView.getComputedStyle(element);
      const box = element.getBoundingClientRect();
      rendered.set(element, style.visibility === 'visible' && box.width > 0 && box.height > 0
        ? (element.innerText || '').trim() : '');
    }
    return rendered.get(element);
  };
  const matches = element => {
    if (kind === 'label') {
      const labelledBy = element.getAttribute('aria-labelledby');
      if (labelledBy) {
        const root = element.getRootNode();
        const labels = labelledBy.trim().split(/\s+/).map(id => root.getElementById(id)).filter(Boolean);
        if (labels.length) return match(labels.map(label => label.textContent || '').join(' ').trim());
      }
      return Array.from(element.labels || []).some(label => match((label.textContent || '').trim()));
    }
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
