// An element's ARIA role as Playwright computes it (its roleUtils.ts
// getAriaRole): the first valid token of the role attribute, else the role
// its tag implies, with presentation inherited from list and table parents
// and a presentation role overridden where the element must stay exposed.
// The page functions that need a role share this one: called once, it
// answers `role(element)` and `nativelyDisabled(element)`, which the role's
// focusability rule also uses.
function() {
  const tag = node => node.localName.toUpperCase();
  const parent = node => node.parentElement ||
    (node.parentNode && node.parentNode.nodeType === 11 ? node.parentNode.host : null);
  const closestCrossShadow = (node, css) => {
    while (node) {
      const closest = node.closest(css);
      if (closest) return closest;
      node = node.getRootNode().host || null;
    }
    return null;
  };
  const validRoles = ['alert', 'alertdialog', 'application', 'article', 'banner', 'blockquote', 'button', 'caption', 'cell', 'checkbox', 'code', 'columnheader', 'combobox', 'complementary', 'contentinfo', 'definition', 'deletion', 'dialog', 'directory', 'document', 'emphasis', 'feed', 'figure', 'form', 'generic', 'grid', 'gridcell', 'group', 'heading', 'img', 'insertion', 'link', 'list', 'listbox', 'listitem', 'log', 'main', 'mark', 'marquee', 'math', 'meter', 'menu', 'menubar', 'menuitem', 'menuitemcheckbox', 'menuitemradio', 'navigation', 'none', 'note', 'option', 'paragraph', 'presentation', 'progressbar', 'radio', 'radiogroup', 'region', 'row', 'rowgroup', 'rowheader', 'scrollbar', 'search', 'searchbox', 'separator', 'slider', 'spinbutton', 'status', 'strong', 'subscript', 'superscript', 'switch', 'tab', 'table', 'tablist', 'tabpanel', 'term', 'textbox', 'time', 'timer', 'toolbar', 'tooltip', 'tree', 'treegrid', 'treeitem'];
  const explicitRole = node => (node.getAttribute('role') || '').split(' ')
    .map(role => role.trim()).find(role => validRoles.includes(role)) || null;
  const namedExplicitly = node => node.hasAttribute('aria-label') || node.hasAttribute('aria-labelledby');
  const landmarkAncestors = 'article:not([role]), aside:not([role]), main:not([role]), nav:not([role]), section:not([role]), [role=article], [role=complementary], [role=main], [role=navigation], [role=region]';
  const globalAttributes = [
    ['aria-atomic'], ['aria-busy'], ['aria-controls'], ['aria-current'], ['aria-describedby'],
    ['aria-details'], ['aria-dropeffect'], ['aria-flowto'], ['aria-grabbed'], ['aria-hidden'],
    ['aria-keyshortcuts'],
    ['aria-label', ['caption', 'code', 'deletion', 'emphasis', 'generic', 'insertion', 'paragraph', 'presentation', 'strong', 'subscript', 'superscript']],
    ['aria-labelledby', ['caption', 'code', 'deletion', 'emphasis', 'generic', 'insertion', 'paragraph', 'presentation', 'strong', 'subscript', 'superscript']],
    ['aria-live'], ['aria-owns'], ['aria-relevant'], ['aria-roledescription', ['generic']],
  ];
  const hasGlobalAttribute = (node, role) => globalAttributes.some(([attribute, prohibited]) =>
    !(prohibited || []).includes(role || '') && node.hasAttribute(attribute));
  const hasTabIndex = node => !Number.isNaN(Number(String(node.getAttribute('tabindex'))));
  const nativelyDisabled = node => ['BUTTON', 'INPUT', 'SELECT', 'TEXTAREA', 'OPTION', 'OPTGROUP'].includes(tag(node)) &&
    (node.hasAttribute('disabled') ||
      (tag(node) === 'OPTION' && !!node.closest('OPTGROUP[DISABLED]')) ||
      (() => {
        const fieldset = node.closest('FIELDSET[DISABLED]');
        if (!fieldset) return false;
        const legend = fieldset.querySelector(':scope > LEGEND');
        return !legend || !legend.contains(node);
      })());
  const nativelyFocusable = node => {
    if (['BUTTON', 'DETAILS', 'SELECT', 'TEXTAREA'].includes(tag(node))) return true;
    if (tag(node) === 'A' || tag(node) === 'AREA') return node.hasAttribute('href');
    if (tag(node) === 'INPUT') return !node.hidden;
    return false;
  };
  const focusable = node => !nativelyDisabled(node) && (nativelyFocusable(node) || hasTabIndex(node));
  const presentationConflict = (node, role) => hasGlobalAttribute(node, role) || focusable(node);
  const inputRoles = {button: 'button', checkbox: 'checkbox', image: 'button', number: 'spinbutton', radio: 'radio', range: 'slider', reset: 'button', submit: 'button'};
  const headerCell = node => !!node && tag(node) === 'TH';
  const nonEmptyDataCell = node => !!node && tag(node) === 'TD' && !!((node.textContent || '').trim() || node.children.length > 0);
  const implicitRoles = {
    A: node => node.hasAttribute('href') ? 'link' : null,
    AREA: node => node.hasAttribute('href') ? 'link' : null,
    ARTICLE: () => 'article', ASIDE: () => 'complementary', BLOCKQUOTE: () => 'blockquote',
    BUTTON: () => 'button', CAPTION: () => 'caption', CODE: () => 'code', DATALIST: () => 'listbox',
    DD: () => 'definition', DEL: () => 'deletion', DETAILS: () => 'group', DFN: () => 'term',
    DIALOG: () => 'dialog', DT: () => 'term', EM: () => 'emphasis', FIELDSET: () => 'group',
    FIGURE: () => 'figure',
    FOOTER: node => closestCrossShadow(node, landmarkAncestors) ? null : 'contentinfo',
    FORM: node => namedExplicitly(node) ? 'form' : null,
    H1: () => 'heading', H2: () => 'heading', H3: () => 'heading', H4: () => 'heading', H5: () => 'heading', H6: () => 'heading',
    HEADER: node => closestCrossShadow(node, landmarkAncestors) ? null : 'banner',
    HR: () => 'separator', HTML: () => 'document',
    IMG: node => node.getAttribute('alt') === '' && !node.getAttribute('title') && !hasGlobalAttribute(node) && !hasTabIndex(node) ? 'presentation' : 'img',
    INPUT: node => {
      const type = node.type.toLowerCase();
      if (['email', 'search', 'tel', 'text', 'url', ''].includes(type)) {
        const id = (node.getAttribute('list') || '').split(' ').find(Boolean);
        let list = null;
        try {
          list = id ? node.getRootNode().querySelector('#' + CSS.escape(id)) : null;
        } catch {}
        if (list && tag(list) === 'DATALIST') return 'combobox';
        return type === 'search' ? 'searchbox' : 'textbox';
      }
      if (type === 'hidden') return null;
      if (type === 'file') return 'button';
      return inputRoles[type] || 'textbox';
    },
    INS: () => 'insertion', LI: () => 'listitem', MAIN: () => 'main', MARK: () => 'mark',
    MATH: () => 'math', MENU: () => 'list', METER: () => 'meter', NAV: () => 'navigation',
    OL: () => 'list', OPTGROUP: () => 'group', OPTION: () => 'option', OUTPUT: () => 'status',
    P: () => 'paragraph', PROGRESS: () => 'progressbar', SEARCH: () => 'search',
    SECTION: node => namedExplicitly(node) ? 'region' : null,
    SELECT: node => node.hasAttribute('multiple') || node.size > 1 ? 'listbox' : 'combobox',
    STRONG: () => 'strong', SUB: () => 'subscript', SUP: () => 'superscript', SVG: () => 'img',
    TABLE: () => 'table', TBODY: () => 'rowgroup',
    TD: node => {
      const table = closestCrossShadow(node, 'table');
      const role = table ? explicitRole(table) : '';
      return role === 'grid' || role === 'treegrid' ? 'gridcell' : 'cell';
    },
    TEXTAREA: () => 'textbox', TFOOT: () => 'rowgroup',
    TH: node => {
      const scope = node.getAttribute('scope');
      if (scope === 'col' || scope === 'colgroup') return 'columnheader';
      if (scope === 'row' || scope === 'rowgroup') return 'rowheader';
      const next = node.nextElementSibling;
      const previous = node.previousElementSibling;
      const row = node.parentElement && tag(node.parentElement) === 'TR' ? node.parentElement : null;
      if (!next && !previous) {
        if (row) {
          const table = closestCrossShadow(row, 'table');
          if (table && table.rows.length <= 1) return null;
        }
        return 'columnheader';
      }
      if (headerCell(next) && headerCell(previous)) return 'columnheader';
      if (nonEmptyDataCell(next) || nonEmptyDataCell(previous)) return 'rowheader';
      return 'columnheader';
    },
    THEAD: () => 'rowgroup', TIME: () => 'time', TR: () => 'row', UL: () => 'list',
  };
  const presentationParents = {DD: ['DL', 'DIV'], DIV: ['DL'], DT: ['DL', 'DIV'], LI: ['OL', 'UL'], TBODY: ['TABLE'], TD: ['TR'], TFOOT: ['TABLE'], TH: ['TR'], THEAD: ['TABLE'], TR: ['THEAD', 'TBODY', 'TFOOT', 'TABLE']};
  const implicitRole = node => {
    const role = implicitRoles[tag(node)]?.(node) || '';
    if (!role) return null;
    for (let ancestor = node; ancestor;) {
      const up = parent(ancestor);
      const parents = presentationParents[tag(ancestor)];
      if (!parents || !up || !parents.includes(tag(up))) break;
      const upRole = explicitRole(up);
      if ((upRole === 'none' || upRole === 'presentation') && !presentationConflict(up, upRole)) return upRole;
      ancestor = up;
    }
    return role;
  };
  const role = element => {
    const explicit = explicitRole(element);
    if (!explicit) return implicitRole(element);
    if (explicit === 'none' || explicit === 'presentation') {
      const implicit = implicitRole(element);
      if (presentationConflict(element, implicit)) return implicit;
    }
    return explicit;
  };
  return {role, nativelyDisabled};
}
