async function(conditions, scroll, probe, cancel) {
  if (cancel) {
    this[probe]?.();
    return null;
  }
  const element = this;
  const view = element.ownerDocument.defaultView;
  // Walk the composed ancestry, including closed and user-agent shadow roots.
  const parent = node => node.parentElement || node.getRootNode().host || null;
  const describe = node => node
    ? `<${node.localName}${node.id ? ` id="${node.id}"` : ''}${node.className && typeof node.className === 'string' ? ` class="${node.className}"` : ''}>`
    : 'no element at the pointer point';
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
    element.scrollIntoView({block: 'center', inline: 'center', behavior: 'instant'});
    for (let frame = view.frameElement; frame; frame = frame.ownerDocument.defaultView.frameElement) {
      frame.scrollIntoView({block: 'center', inline: 'center', behavior: 'instant'});
    }
  }
  let box = element.getBoundingClientRect();
  let stable = !conditions.includes('stable');
  if (!stable) {
    // Chromiumoxide cannot cancel an awaited page function. The caller owns
    // this temporary cancellation callback and invokes it on every failed wait.
    let previousFrame = null;
    try {
      for (let count = 0; count < 10; count++) {
        const observed = await new Promise(resolve => {
          const frame = view.requestAnimationFrame(() => resolve(true));
          Object.defineProperty(element, probe, {configurable: true, value: () => {
            view.cancelAnimationFrame(frame);
            resolve(false);
          }});
        });
        if (!observed) break;
        const next = element.getBoundingClientRect();
        stable = previousFrame !== null && ['x', 'y', 'width', 'height'].every(key => previousFrame[key] === next[key]);
        previousFrame = next;
        box = next;
        if (stable) break;
      }
    } finally {
      delete element[probe];
    }
  }

  // Recompute all conditions after scrolling and the animation-frame observation.
  result.attached = element.isConnected;
  result.checked = checkable ? (nativeCheck ? element.checked : element.getAttribute('aria-checked') === 'true') : null;
  result.radio = nativeCheck ? element.type === 'radio' : role === 'radio';
  const style = view.getComputedStyle(element);
  result.visible = result.attached && box.width > 0 && box.height > 0 && style.visibility === 'visible';
  result.enabled = result.attached;
  for (let node = element; node; node = parent(node)) {
    if (node.matches(':disabled') || node.getAttribute('aria-disabled') === 'true') {
      result.enabled = false;
      break;
    }
  }
  const editable = editType && !element.readOnly && element.getAttribute('aria-readonly') !== 'true';
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
    // Verify each root from the target outwards. Accessing getRootNode() on the
    // target also exposes its closed/native root, unlike host.shadowRoot.
    let target = element;
    let x = result.x;
    let y = result.y;
    while (target) {
      const root = target.getRootNode();
      const hit = root.elementFromPoint(x, y);
      let ancestor = hit;
      while (ancestor && ancestor !== target) ancestor = parent(ancestor);
      if (!ancestor) return {...result, failed: 'hit', interceptor: describe(hit)};
      if (root.host) {
        target = root.host;
      } else {
        const frame = target.ownerDocument.defaultView.frameElement;
        if (!frame) break;
        const rect = frame.getBoundingClientRect();
        x += rect.left + frame.clientLeft;
        y += rect.top + frame.clientTop;
        target = frame;
      }
    }
  }
  for (let frame = view.frameElement; frame; frame = frame.ownerDocument.defaultView.frameElement) {
    const rect = frame.getBoundingClientRect();
    result.x += rect.left + frame.clientLeft;
    result.y += rect.top + frame.clientTop;
  }
  return result;
}
