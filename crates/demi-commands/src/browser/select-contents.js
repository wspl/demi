function() {
  if (this.isContentEditable) {
    const range = document.createRange();
    range.selectNodeContents(this);
    const selection = this.ownerDocument.getSelection();
    selection.removeAllRanges();
    selection.addRange(range);
  } else {
    this.select();
  }
}
