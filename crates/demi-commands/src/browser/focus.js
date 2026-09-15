function(focus) {
  if (!this.isConnected) return false;
  if (focus) this.focus({preventScroll: true});
  let node = this;
  while (node) {
    const root = node.getRootNode();
    if (root.activeElement !== node) return false;
    node = root.host || node.ownerDocument.defaultView.frameElement;
  }
  return true;
}
