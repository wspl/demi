function(text, apply, native) {
  if (native) {
    // Validate on a detached native control before touching the target or its focus.
    const probe = this.cloneNode(false);
    const setter = Object.getOwnPropertyDescriptor(this.ownerDocument.defaultView.HTMLInputElement.prototype, 'value').set;
    setter.call(probe, text);
    if (probe.value !== text) return 'invalid';
    if (apply) {
      this.focus({preventScroll: true});
      setter.call(this, text);
      this.dispatchEvent(new Event('input', {bubbles: true, composed: true}));
      this.dispatchEvent(new Event('change', {bubbles: true}));
    }
    return 'native';
  }
  if (this.localName === 'input' && this.type === 'number' && text !== '' &&
      (!text.trim() || !Number.isFinite(Number(text)))) return 'invalid';
  if (apply) {
    this.focus({preventScroll: true});
    if (this.localName === 'input' || this.localName === 'textarea') {
      this.select();
    } else {
      const range = this.ownerDocument.createRange();
      range.selectNodeContents(this);
      const selection = this.ownerDocument.defaultView.getSelection();
      selection.removeAllRanges();
      selection.addRange(range);
    }
  }
  return 'text';
}
