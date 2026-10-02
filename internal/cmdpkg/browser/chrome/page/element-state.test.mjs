import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import test from 'node:test';

const source = readFileSync(new URL('./element-state.js', import.meta.url), 'utf8');
const elementState = new Function(`return (${source})`)();

// Supply browser geometry and explicitly deliver frames. No Chrome or clock waits.
function animatedElement(hidden = false) {
  let callback;
  let x = 0;
  const view = {
    document: {visibilityState: hidden ? 'hidden' : 'visible'},
    innerWidth: 1280,
    innerHeight: 720,
    requestAnimationFrame(next) {
      callback = next;
      return 1;
    },
    cancelAnimationFrame() { callback = undefined; },
    setTimeout(next) {
      callback = next;
      return 1;
    },
    clearTimeout() { callback = undefined; },
    getComputedStyle() { return {visibility: 'visible'}; },
  };
  return {
    element: {
      ownerDocument: {defaultView: view},
      localName: 'button',
      isConnected: true,
      getAttribute() { return null; },
      getRootNode() { return {}; },
      matches() { return false; },
      getBoundingClientRect() { return {x, y: 0, width: 20, height: 20, left: x, right: x + 20, top: 0, bottom: 20}; },
    },
    async frame(position) {
      assert.ok(callback, 'the probe must request its next sample');
      x = position;
      const next = callback;
      callback = undefined;
      next();
      await Promise.resolve();
    },
    pending() { return callback !== undefined; },
  };
}

for (const hidden of [false, true]) {
  test(`moving geometry is refused and releases its probe (${hidden ? 'hidden' : 'visible'})`, async () => {
    const fixture = animatedElement(hidden);
    const result = elementState.call(fixture.element, ['stable'], false, 'probe_test', false);
    for (let frame = 0; frame < 10; frame++) {
      await fixture.frame(frame * 10);
    }
    assert.equal((await result).failed, 'stable');
    assert.equal(Object.hasOwn(fixture.element, 'probe_test'), false);
    assert.equal(fixture.pending(), false);
  });

  test(`stability needs two unchanged frame intervals (${hidden ? 'hidden' : 'visible'})`, async () => {
    const fixture = animatedElement(hidden);
    const result = elementState.call(fixture.element, ['stable'], false, 'probe_test', false);
    await fixture.frame(0);
    assert.equal(fixture.pending(), true, 'the pre-probe box is not an animation sample');
    await fixture.frame(10);
    assert.equal(fixture.pending(), true, 'movement must restart the comparison');
    await fixture.frame(10);
    assert.equal(fixture.pending(), true, 'one equal pair can straddle an animation reversal');
    await fixture.frame(10);
    assert.equal((await result).failed, null);
    assert.equal(Object.hasOwn(fixture.element, 'probe_test'), false);
  });

  test(`an animation reversal is not a stopped box (${hidden ? 'hidden' : 'visible'})`, async () => {
    const fixture = animatedElement(hidden);
    const result = elementState.call(fixture.element, ['stable'], false, 'probe_test', false);
    // Chrome can round the positions on opposite sides of a reversal to the
    // same CSS pixel fraction, even on distinct consecutive animation frames.
    for (const x of [154.65625, 182.328125, 210.15625, 210.15625, 182.328125,
      154.65625, 126.828125, 99, 71.171875, 43.34375]) {
      await fixture.frame(x);
    }
    assert.equal((await result).failed, 'stable');
    assert.equal(Object.hasOwn(fixture.element, 'probe_test'), false);
    assert.equal(fixture.pending(), false);
  });

  test(`cancellation releases the pending sample (${hidden ? 'hidden' : 'visible'})`, async () => {
    const fixture = animatedElement(hidden);
    const result = elementState.call(fixture.element, ['stable'], false, 'probe_test', false);
    await elementState.call(fixture.element, [], false, 'probe_test', true);
    assert.equal((await result).failed, 'stable');
    assert.equal(fixture.pending(), false);
    assert.equal(Object.hasOwn(fixture.element, 'probe_test'), false);
  });
}
