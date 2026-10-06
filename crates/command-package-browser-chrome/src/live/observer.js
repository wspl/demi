// The live view's page observer (`live-view.md` § Input). It runs in
// an isolated world of each watched tab's documents, where the page can
// neither see nor call it, and reports through the world's own binding: the
// cursors of the visible page and the one under the viewer's pointer, the
// page's native form controls, and text the page copies. The module calls `globalThis.demiLive` to apply a
// viewer's choice to a control.
(() => {
  if (globalThis.demiLive) return;
  const report = message => {
    try {
      globalThis.demiLiveReport(JSON.stringify(message));
    } catch {
      // The binding goes away with the document.
    }
  };
  // Page strings within the protocol's lengths, as valid UTF-16.
  const clip = (value, length) => {
    const text = String(value ?? '').toWellFormed();
    if (text.length <= length) return text;
    const end = /[\uD800-\uDBFF]/.test(text[length - 1]) ? length - 1 : length;
    return text.slice(0, end);
  };
  const top = window === window.top;

  // The cursors of the visible page (`live-view.md` § Input). Each frame
  // reports where each cursor applies, so the view resolves the cursor under
  // its pointer without asking: a region is an element whose cursor differs
  // from its parent's, in document order,
  // so a later region over an earlier one wins as a child's cursor does. A
  // region of `auto` leaves the cursor to the browser, which this observer
  // resolves for the point the viewer's pointer is at: a text cursor over
  // text, as a local browser does.
  const EDITABLE = 'textarea, input:not([type]), input[type=text], input[type=search], input[type=url], input[type=tel], input[type=email], input[type=password], input[type=number]';
  const MAX_REGIONS = 2000;
  // A cursor list falls back to its last keyword, the one every browser has.
  const keyword = cursor => cursor.split(',').pop().trim();
  const editable = element => element.matches(EDITABLE) || element.isContentEditable === true;
  // Each frame reports in its own viewport's CSS pixels; the module, which
  // knows where every frame stands, cross-site ones in their own processes
  // included, places them in the tab. A shadow root's elements count as the
  // page's, under their host.
  const regions = () => {
    const result = [];
    const viewport = { left: 0, top: 0, right: innerWidth, bottom: innerHeight };
    // Each element with the cursor it passes on and the box its children
    // show within: a scrolled or clipped container hides what lies outside it.
    const pending = [[document.documentElement, { cursor: 'auto', clip: viewport }]];
    while (pending.length > 0 && result.length < MAX_REGIONS) {
      const [element, parent] = pending.pop();
      const style = getComputedStyle(element);
      let cursor = keyword(style.cursor);
      if (cursor === 'auto' && editable(element)) cursor = style.writingMode.startsWith('vertical') ? 'vertical-text' : 'text';
      const shown = style.position === 'fixed' ? viewport : parent.clip;
      let within = shown;
      if (style.overflowX !== 'visible' || style.overflowY !== 'visible') {
        const box = element.getBoundingClientRect();
        within = {
          left: Math.max(shown.left, box.left),
          top: Math.max(shown.top, box.top),
          right: Math.min(shown.right, box.right),
          bottom: Math.min(shown.bottom, box.bottom),
        };
      }
      // An element the pointer passes through shows no cursor of its own, as
      // a hidden backdrop that covers the whole page does not: its children
      // compare with the cursor shown around it, so a visible child that
      // inherits the hidden element's cursor is a region of its own.
      const hit = style.pointerEvents !== 'none' && style.visibility === 'visible';
      if (!hit) cursor = parent.cursor;
      if (cursor !== parent.cursor) {
        for (const rect of element.getClientRects()) {
          const left = Math.max(rect.left, shown.left);
          const upper = Math.max(rect.top, shown.top);
          const right = Math.min(rect.right, shown.right);
          const bottom = Math.min(rect.bottom, shown.bottom);
          if (right <= left || bottom <= upper) continue;
          result.push({ x: left, y: upper, width: right - left, height: bottom - upper, cursor: clip(cursor, 200) });
        }
      }
      // Children in document order: the stack takes them last first.
      const passed = { cursor, clip: within };
      const children = [...element.children, ...(element.shadowRoot?.children ?? [])];
      for (let index = children.length - 1; index >= 0; index--) pending.push([children[index], passed]);
    }
    return result;
  };
  // The cursor at `point` as the browser shows it, `auto` resolved.
  const cursorAt = point => {
    let element = document.elementFromPoint(point.x, point.y);
    const shadowRoots = [];
    while (element?.shadowRoot) {
      shadowRoots.push(element.shadowRoot);
      const inner = element.shadowRoot.elementFromPoint(point.x, point.y);
      if (!inner || inner === element) break;
      element = inner;
    }
    if (!element || element.matches('iframe,frame')) return null;
    const style = getComputedStyle(element);
    const field = editable(element);
    let cursor = keyword(style.cursor);
    if (cursor === 'auto') {
      let textHit = false;
      if (style.userSelect !== 'none' && !element.closest('button,select,input')) {
        const caret = document.caretPositionFromPoint(point.x, point.y, { shadowRoots });
        if (caret?.offsetNode.nodeType === Node.TEXT_NODE && element.contains(caret.offsetNode)) {
          const range = document.createRange();
          const node = caret.offsetNode;
          for (const offset of [caret.offset - 1, caret.offset]) {
            if (offset < 0 || offset >= node.length) continue;
            range.setStart(node, offset);
            range.setEnd(node, offset + 1);
            textHit = Array.from(range.getClientRects()).some(rect =>
              point.x >= rect.left && point.x <= rect.right && point.y >= rect.top && point.y <= rect.bottom);
            if (textHit) break;
          }
        }
      }
      cursor = field || textHit ? (style.writingMode.startsWith('vertical') ? 'vertical-text' : 'text') : 'default';
    }
    // An SVG element has no isContentEditable; an undefined field would be dropped from the report.
    return { cursor: clip(cursor, 200), editable: field };
  };
  // The pointer in this frame's viewport, from the page's own events or, in
  // a new document under a still pointer, from the module.
  let point;
  // Something on the page may have moved since the cursors were reported.
  let dirty = true;
  let lastRegions = '';
  let lastCursor = '';
  const touched = () => {
    dirty = true;
  };
  // The pointer moving in this frame always reports what it resolves: the
  // tab has one cursor, which another frame's observer may have reported
  // since this one last did. Only a resolution at a still pointer that this
  // observer reported already is left out.
  const reportCursors = moved => {
    if (!dirty || !document.documentElement) return;
    dirty = false;
    const found = regions();
    const serialized = JSON.stringify(found);
    if (serialized !== lastRegions) {
      lastRegions = serialized;
      report({ type: 'cursors', regions: found });
    }
    if (!point) return;
    const resolved = cursorAt(point);
    const key = JSON.stringify(resolved);
    if (resolved && (moved || key !== lastCursor)) {
      lastCursor = key;
      report({ type: 'cursor', ...resolved });
    }
  };
  new MutationObserver(touched).observe(document, { subtree: true, childList: true, attributes: true, characterData: true });
  for (const type of ['scroll', 'resize', 'transitionend', 'animationend', 'load']) {
    addEventListener(type, touched, { passive: true, capture: true });
  }
  addEventListener('pointermove', event => {
    point = { x: event.clientX, y: event.clientY };
    // Hovering changes styles, and the cursor at the point is what the view asks about.
    dirty = true;
    reportCursors(true);
  }, { passive: true, capture: true });
  setInterval(() => {
    try {
      reportCursors(false);
    } catch {
      // A document being replaced; the next one reports its own.
    }
  }, 200);

  // Native controls the viewer opens with its own pickers: each has an
  // identity and a revision that changes whenever what it reports changes.
  const KINDS = ['select', 'date', 'month', 'week', 'time', 'datetime-local', 'color', 'suggestions', 'file'];
  const identities = new WeakMap();
  const controls = new Map();
  let lastControls = '';
  const snapshot = () => {
    const result = [];
    const present = new Set();
    for (const element of document.querySelectorAll('select,input')) {
      const kind = element.tagName === 'SELECT' ? 'select' : element.list ? 'suggestions' : element.type;
      if (!KINDS.includes(kind)) continue;
      const rect = element.getBoundingClientRect();
      if (!element.checkVisibility() || rect.width <= 0 || rect.height <= 0 || rect.bottom <= 0 ||
        rect.top >= innerHeight || rect.right <= 0 || rect.left >= innerWidth) continue;
      let token = identities.get(element);
      if (!token) {
        token = crypto.randomUUID();
        identities.set(element, token);
      }
      const options = Array.from(kind === 'select' ? element.options : (element.list?.options ?? []), option => {
        const group = option.parentElement?.tagName === 'OPTGROUP' ? option.parentElement : null;
        return {
          label: clip(option.label, 2000),
          value: clip(option.value, 2000),
          group: clip(group?.label, 2000),
          disabled: option.disabled || Boolean(group?.disabled),
          hidden: option.hidden || getComputedStyle(option).display === 'none' || (group !== null && getComputedStyle(group).display === 'none'),
          selected: option.selected,
        };
      }).slice(0, 1000);
      const data = {
        kind,
        label: clip(element.labels?.[0]?.textContent.trim() || element.getAttribute('aria-label') || element.id || kind, 2000),
        value: clip(element.value, 10000),
        min: clip(element.min, 100),
        max: clip(element.max, 100),
        step: clip(element.step, 100),
        accept: clip(element.accept, 1000),
        multiple: Boolean(element.multiple),
        disabled: element.disabled || Boolean(element.readOnly),
        required: element.required,
        size: kind === 'select' ? element.size : 0,
        options,
      };
      const signature = JSON.stringify(data);
      const old = controls.get(token);
      const revision = old ? old.revision + Number(old.signature !== signature) : 0;
      controls.set(token, { element, signature, revision, committed: old?.committed });
      present.add(token);
      result.push({ token, revision, ...data, rect: { x: rect.x, y: rect.y, width: rect.width, height: rect.height } });
      if (result.length === 100) break;
    }
    for (const [token, control] of controls) {
      if (!present.has(token)) {
        identities.delete(control.element);
        controls.delete(token);
      }
    }
    return result;
  };
  // A choice applies to the revision the viewer saw, or to the one its own
  // earlier choice produced, so fast typing into a suggestion field holds.
  const element = ({ token, revision }) => {
    snapshot();
    const control = controls.get(token);
    const current = control?.revision === revision ||
      (control?.committed?.from === revision && control.committed.to === control.revision);
    return current && !control.element.disabled && !control.element.readOnly ? control.element : null;
  };
  const committed = ({ token, revision }) => {
    snapshot();
    const control = controls.get(token);
    if (control) control.committed = { from: revision, to: control.revision };
  };
  const commit = message => {
    const target = element(message);
    if (!target || target.type === 'file') return false;
    if (target.tagName === 'SELECT') {
      if (!target.multiple && message.indices.length !== 1) return false;
      const options = Array.from(target.options);
      if (message.indices.some(index => !options[index] || options[index].disabled || options[index].parentElement.disabled || options[index].hidden)) return false;
      for (let index = 0; index < options.length; index++) options[index].selected = message.indices.includes(index);
    } else {
      const check = target.cloneNode();
      check.required = false;
      check.value = message.value;
      if (check.value !== message.value || !check.validity.valid) return false;
      target.value = message.value;
    }
    target.focus({ preventScroll: true });
    // The page hears the choice as the user's own edit.
    target.dispatchEvent(new Event('input', { bubbles: true, composed: true }));
    target.dispatchEvent(new Event('change', { bubbles: true }));
    committed(message);
    return true;
  };

  // Text the page copies, once: a copy both fires its event and changes the
  // clipboard.
  let lastCopy;
  const copied = text => {
    if (!text) return;
    const now = performance.now();
    if (lastCopy?.text === text && now - lastCopy.at < 1000) return;
    lastCopy = { text, at: now };
    report({ type: 'copy', text: clip(text, 1000000) });
  };
  // What a copy or cut takes: the selection, or what the page's own handler set.
  const selected = () => {
    const active = document.activeElement;
    if (active instanceof HTMLInputElement && active.type === 'password') return null;
    if (active instanceof HTMLTextAreaElement || active instanceof HTMLInputElement) {
      try {
        if (active.selectionEnd > active.selectionStart) return active.value.slice(active.selectionStart, active.selectionEnd);
      } catch {
        // Some input types have no selection range.
      }
    }
    return String(getSelection() ?? '');
  };
  for (const type of ['copy', 'cut']) {
    addEventListener(type, () => {
      let taken = selected();
      if (taken === null) return;
      // Added during dispatch, this listener runs after the page's own.
      addEventListener(type, event => {
        if (event.defaultPrevented) taken = event.clipboardData?.getData('text/plain') ?? '';
      }, { once: true });
      setTimeout(() => copied(taken), 0);
    }, true);
  }

  if (top) {
    // What the page writes with the Clipboard API; the browser lets this
    // world read it only where its clipboard is its own.
    navigator.clipboard?.addEventListener?.('clipboardchange', () => {
      navigator.clipboard.readText().then(copied, () => {});
    });
    setInterval(() => {
      let current;
      try {
        current = snapshot();
      } catch {
        return;
      }
      const serialized = JSON.stringify(current);
      if (serialized === lastControls) return;
      lastControls = serialized;
      report({ type: 'controls', controls: current });
    }, 200);
  }
  // The viewer's pointer in a new top document, which no pointer event reached yet.
  const pointer = (x, y) => {
    point = { x, y };
    dirty = true;
  };
  globalThis.demiLive = { element, committed, commit, pointer };
})();
