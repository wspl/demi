function(text, cursor, prefix, suffix) {
  const input = this.localName === 'input' || this.localName === 'textarea';
  const nodes = [];
  let content = '';
  if (input) {
    if (this.selectionStart === null) return {status: 'unsupported', count: 0};
    content = this.value;
  } else {
    const walker = this.ownerDocument.createTreeWalker(this, NodeFilter.SHOW_TEXT);
    while (walker.nextNode()) {
      const node = walker.currentNode;
      const range = this.ownerDocument.createRange();
      range.selectNodeContents(node);
      const style = node.parentElement && getComputedStyle(node.parentElement);
      if (!style || style.visibility !== 'visible' || !range.getClientRects().length) continue;
      nodes.push({node, start: content.length});
      content += node.textContent;
    }
  }
  const matches = [];
  for (let at = content.indexOf(text); at >= 0; at = content.indexOf(text, at + 1)) {
    if ((prefix === null || content.slice(0, at).endsWith(prefix)) &&
        (suffix === null || content.slice(at + text.length).startsWith(suffix))) matches.push(at);
  }
  if (matches.length !== 1) return {status: matches.length ? 'ambiguous' : 'missing', count: matches.length};
  let start = matches[0];
  let end = start + text.length;
  if (cursor === 'before') end = start;
  if (cursor === 'after') start = end;
  this.focus({preventScroll: true});
  if (input) {
    this.setSelectionRange(start, end);
  } else {
    const boundary = at => {
      const entry = nodes.find(entry => at <= entry.start + entry.node.textContent.length);
      return [entry.node, at - entry.start];
    };
    const range = this.ownerDocument.createRange();
    range.setStart(...boundary(start));
    range.setEnd(...boundary(end));
    const selection = this.ownerDocument.getSelection();
    selection.removeAllRanges();
    selection.addRange(range);
  }
  return {status: 'selected', count: 1};
}
