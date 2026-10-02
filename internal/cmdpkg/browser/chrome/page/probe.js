function(x, y, includeOrdinary) {
  if (x < 0 || y < 0 || x >= innerWidth || y >= innerHeight) return [];
  const candidates = [];
  const seen = new Set();
  const visit = root => {
    for (const element of root.elementsFromPoint(x, y)) {
      if (seen.has(element)) continue;
      seen.add(element);
      if (element.shadowRoot) visit(element.shadowRoot);
      const interactive = element.matches('button,input,select,textarea,a[href],summary,[contenteditable="true"],[tabindex]') ||
        ['button','link','checkbox','radio','switch','textbox','combobox','slider','spinbutton','menuitem','option','tab','treeitem'].includes(element.getAttribute('role'));
      if (includeOrdinary || interactive) candidates.push(element);
    }
  };
  visit(document);
  return candidates;
}
