function(x, y, editable) {
  if (!this.isConnected) {
    return false;
  }
  const style = getComputedStyle(this);
  if (style.visibility !== 'visible' || style.display === 'none') {
    return false;
  }
  if (this.matches(':disabled') || this.getAttribute('aria-disabled') === 'true') {
    return false;
  }
  if (editable && !(this.isContentEditable ||
      ((this instanceof HTMLInputElement || this instanceof HTMLTextAreaElement) &&
       !this.readOnly))) return false;
  let hit = this.ownerDocument.elementFromPoint(x, y);
  while (hit && hit.shadowRoot) {
    const inner = hit.shadowRoot.elementFromPoint(x, y);
    if (!inner || inner === hit) {
      break;
    }
    hit = inner;
  }
  return hit === this || this.contains(hit);
}
