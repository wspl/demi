function(kind, expected, exact) {
  const match = text => exact ? text === expected : text.includes(expected);
  if (kind === 'label') {
    const labelledBy = this.getAttribute('aria-labelledby');
    if (labelledBy) {
      const root = this.getRootNode();
      const labels = labelledBy.trim().split(/\s+/).map(id => root.getElementById(id)).filter(Boolean);
      if (labels.length) return match(labels.map(label => label.textContent || '').join(' ').trim());
    }
    return Array.from(this.labels || []).some(label => match((label.textContent || '').trim()));
  }
  // innerText is rendered text, but on a hidden element it falls back to raw text.
  const visibleText = element => {
    const style = element.ownerDocument.defaultView.getComputedStyle(element);
    const box = element.getBoundingClientRect();
    return style.visibility === 'visible' && box.width > 0 && box.height > 0
      ? (element.innerText || '').trim() : '';
  };
  const text = visibleText(this);
  if (!text || !match(text)) return false;
  // Prefer the smallest rendered text owner, rather than matching every ancestor.
  return !Array.from(this.children).some(child => match(visibleText(child)));
}
