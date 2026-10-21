// The checks before an action, as Playwright makes them
// (packages/injected/src/injectedScript.ts): visible, enabled, editable,
// stable and receiving the pointer. `scroll` is false, true to scroll the
// element into view only when it is not, or an alignment ('start', 'center',
// 'end') that a retry tries in turn, so that an element under a sticky header
// comes out from under it. `intercept` installs Playwright's hit-target
// interceptor under the key `probe`; calling the function with `cancel` and
// that key stops it and answers what it saw. `aria` is the shared role
// resolution the caller composes in (`element.rs`, `ELEMENT_STATE`).
async function(conditions, scroll, probe, cancel, point, intercept) {
  if (cancel) {
    return this[probe]?.() ?? null;
  }
  const element = this;
  const view = element.ownerDocument.defaultView;
  // Walk the composed ancestry, including closed and user-agent shadow roots.
  const parent = node => node.parentElement || node.getRootNode().host || null;
  const describe = node => node
    ? `<${node.localName}${node.id ? ` id="${node.id}"` : ''}${node.className && typeof node.className === 'string' ? ` class="${node.className}"` : ''}>`
    : 'no element at the pointer point';
  // A state of a control's inner part is the control's: 'button-link' for
  // the pointer, 'follow-label' for enabled and editable, as Playwright
  // retargets.
  const retarget = behavior => {
    let node = element;
    if (!node.matches('input, textarea, select') && !node.isContentEditable) {
      node = node.closest(behavior === 'button-link'
        ? 'button, [role=button], a, [role=link]'
        : 'button, [role=button], [role=checkbox], [role=radio]') || node;
    }
    if (behavior === 'follow-label' &&
        !node.matches('a, input, textarea, button, select, [role=link], [role=button], [role=checkbox], [role=radio]') &&
        !node.isContentEditable) {
      const label = node.closest('label');
      if (label && label.control) node = label.control;
    }
    return node;
  };
  const visibleText = node => {
    const range = node.ownerDocument.createRange();
    range.selectNode(node);
    const box = range.getBoundingClientRect();
    return box.width > 0 && box.height > 0;
  };
  const isVisible = node => {
    const style = view.getComputedStyle(node);
    if (style.display === 'contents') {
      for (let child = node.firstChild; child; child = child.nextSibling) {
        if ((child.nodeType === 1 && isVisible(child)) || (child.nodeType === 3 && visibleText(child))) return true;
      }
      return false;
    }
    if (node.checkVisibility && !node.checkVisibility()) return false;
    if (style.visibility !== 'visible') return false;
    const box = node.getBoundingClientRect();
    return box.width > 0 && box.height > 0;
  };
  // The roles aria-disabled applies to; `aria` is the shared role
  // resolution (`aria-role.js`).
  const disabledRoles = ['application', 'button', 'composite', 'gridcell', 'group', 'input', 'link', 'menuitem', 'scrollbar', 'separator', 'tab', 'checkbox', 'columnheader', 'combobox', 'grid', 'listbox', 'menu', 'menubar', 'menuitemcheckbox', 'menuitemradio', 'option', 'radio', 'radiogroup', 'row', 'rowheader', 'searchbox', 'select', 'slider', 'spinbutton', 'switch', 'tablist', 'textbox', 'toolbar', 'tree', 'treegrid', 'treeitem'];
  const isDisabled = node => {
    if (aria.nativelyDisabled(node)) return true;
    if (!disabledRoles.includes(aria.role(node) || '')) return false;
    for (let ancestor = node; ancestor; ancestor = parent(ancestor)) {
      const disabled = (ancestor.getAttribute('aria-disabled') || '').toLowerCase();
      if (disabled === 'true') return true;
      if (disabled === 'false') return false;
    }
    return false;
  };
  // Null when the pointer at (x, y) lands on the element, or what it lands on
  // instead.
  const hitTarget = (x, y) => {
    const target = retarget('button-link');
    const roots = [];
    for (let node = target; node;) {
      const root = node.getRootNode();
      if (root.nodeType !== 9 && root.nodeType !== 11) break;
      roots.push(root);
      if (root.nodeType === 9) break;
      node = root.host;
    }
    let hit;
    for (let index = roots.length - 1; index >= 0; index--) {
      const root = roots[index];
      const elements = root.elementsFromPoint(x, y);
      const single = root.elementFromPoint(x, y);
      if (single && elements[0] && parent(single) === elements[0] && view.getComputedStyle(single).display === 'contents') {
        elements.unshift(single);
      }
      if (elements[0] && elements[0].shadowRoot === root && elements[1] === single) elements.shift();
      if (!elements[0]) break;
      hit = elements[0];
      if (index && hit !== roots[index - 1].host) break;
    }
    const passed = [];
    while (hit && hit !== target) {
      passed.push(hit);
      hit = hit.assignedSlot ?? parent(hit);
    }
    return hit === target ? null : describe(passed[0] || null);
  };

  const input = element.localName === 'input';
  const nativeCheck = input && ['checkbox', 'radio'].includes(element.type);
  const role = element.getAttribute('role');
  const checkable = nativeCheck || ['checkbox', 'radio', 'switch'].includes(role);
  const editType = input || element.localName === 'textarea' || element.isContentEditable;
  const nativeTypes = ['date', 'time', 'datetime-local', 'month', 'week', 'color', 'range'];
  const fillable = element.isContentEditable || element.localName === 'textarea' ||
    (input && (['text', 'email', 'number', 'password', 'search', 'tel', 'url'].includes(element.type) || nativeTypes.includes(element.type)));
  const result = {
    fillKind: !fillable ? 'none' : input && nativeTypes.includes(element.type) ? 'native' : 'text',
    attached: element.isConnected, visible: false, enabled: false,
    checked: null, radio: false,
    failed: null, interceptor: null, permanent: false, x: 0, y: 0,
  };
  for (const condition of conditions) {
    if ((condition === 'fillable' && !fillable) ||
        (condition === 'editable' && !editType) ||
        (condition === 'checkable' && !checkable) ||
        (condition === 'selectable' && element.localName !== 'select')) {
      return {...result, failed: condition, permanent: true};
    }
  }
  if (!element.isConnected) return {...result, failed: 'attached'};
  if (scroll) {
    const reveal = node => scroll === true
      ? node.scrollIntoViewIfNeeded(true)
      : node.scrollIntoView({block: scroll, inline: scroll, behavior: 'instant'});
    reveal(element);
    for (let frame = view.frameElement; frame; frame = frame.ownerDocument.defaultView.frameElement) {
      reveal(frame);
    }
  }
  let box = element.getBoundingClientRect();
  let stable = !conditions.includes('stable');
  if (!stable) {
    // Chromium's rule: the box is the same in two consecutive animation
    // frames; a box that moved fails the check at once, and the action checks
    // again after a pause. Chromiumoxide cannot cancel an awaited page
    // function, so the caller owns this temporary cancellation callback and
    // invokes it on every failed wait.
    const target = retarget('no-follow-label');
    let previous = null;
    try {
      while (true) {
        const observed = await new Promise(resolve => {
          const frame = view.requestAnimationFrame(() => resolve(true));
          Object.defineProperty(element, probe, {configurable: true, value: () => {
            view.cancelAnimationFrame(frame);
            resolve(false);
          }});
        });
        if (!observed || !target.isConnected) break;
        const next = target.getBoundingClientRect();
        if (previous !== null) {
          stable = ['top', 'left', 'width', 'height'].every(key => previous[key] === next[key]);
          break;
        }
        previous = next;
      }
    } finally {
      delete element[probe];
    }
    box = element.getBoundingClientRect();
  }

  // Recompute all conditions after scrolling and the animation-frame observation.
  result.attached = element.isConnected;
  result.checked = checkable ? (nativeCheck ? element.checked : element.getAttribute('aria-checked') === 'true') : null;
  result.radio = nativeCheck ? element.type === 'radio' : role === 'radio';
  const style = view.getComputedStyle(element);
  result.visible = result.attached && isVisible(element);
  result.enabled = result.attached && !isDisabled(retarget('follow-label'));
  const editable = editType && result.enabled && !element.readOnly && element.getAttribute('aria-readonly') !== 'true';
  const left = Math.max(0, box.left);
  const right = Math.min(view.innerWidth, box.right);
  const top = Math.max(0, box.top);
  const bottom = Math.min(view.innerHeight, box.bottom);
  result.x = (left + right) / 2;
  result.y = (top + bottom) / 2;
  const geometry = box.width > 0 && box.height > 0 && right > left && bottom > top;
  const states = {...result, editable, geometry, stable, fillable, checkable, selectable: element.localName === 'select'};
  for (const condition of conditions) {
    if (condition !== 'hit' && !states[condition]) return {...result, failed: condition};
  }
  if (conditions.includes('hit')) {
    if (style.pointerEvents === 'none') return {...result, failed: 'pointer-events', interceptor: describe(element)};
    const [x, y] = point ?? [result.x, result.y];
    const interceptor = hitTarget(x, y);
    if (interceptor !== null) return {...result, failed: 'hit', interceptor};
    if (intercept) {
      // The input's first event decides, as in Playwright: when it lands
      // elsewhere, which a layout shift between this check and the input can
      // cause, every event of the input is stopped before the page sees it and
      // the action checks again. When it lands on the element, nothing more is
      // checked, so the listener leaves at once and stays behind in no case,
      // a dialog the click opened included.
      const types = ['mousedown', 'mouseup', 'pointerdown', 'pointerup', 'click', 'auxclick', 'dblclick', 'contextmenu'];
      let outcome;
      const remove = () => {
        for (const type of types) view.removeEventListener(type, listener, {capture: true});
      };
      const listener = event => {
        if (!event.isTrusted) return;
        if (outcome === undefined) {
          outcome = hitTarget(event.clientX, event.clientY);
          if (outcome === null) {
            remove();
            delete element[probe];
          }
        }
        if (outcome !== null) {
          event.preventDefault();
          event.stopPropagation();
          event.stopImmediatePropagation();
        }
      };
      for (const type of types) view.addEventListener(type, listener, {capture: true, passive: false});
      Object.defineProperty(element, probe, {configurable: true, value: () => {
        remove();
        delete element[probe];
        return outcome ?? null;
      }});
    }
  }
  return result;
}
