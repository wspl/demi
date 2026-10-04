import { expect, test } from 'bun:test'
import { readFile } from 'node:fs/promises'

// Runs the element-state page function, as `element.rs` ships it, on a
// stand-in element whose box the test moves one animation frame at a time.
// `bun run test` runs it.

interface Box {
  x: number
  y: number
  width: number
  height: number
  left: number
  right: number
  top: number
  bottom: number
}

type ElementState = (
  this: object,
  conditions: string[],
  scroll: boolean,
  probe: string,
  cancel: boolean,
) => Promise<{ failed: string | null }>

async function elementState(): Promise<ElementState> {
  const source = await readFile(new URL('./element-state.js', import.meta.url), 'utf8')
  return new Function(`return (${source})`)() as ElementState
}

/** A 20 px button at `x`, and the frame the page function waits for. */
function animated() {
  let x = 0
  let frame: (() => void) | undefined
  const view = {
    document: { visibilityState: 'visible' },
    innerWidth: 1280,
    innerHeight: 720,
    requestAnimationFrame(callback: () => void): number {
      frame = callback
      return 1
    },
    cancelAnimationFrame(): void {
      frame = undefined
    },
    getComputedStyle: () => ({ visibility: 'visible' }),
  }
  const element = {
    ownerDocument: { defaultView: view },
    localName: 'button',
    isConnected: true,
    getAttribute: () => null,
    getRootNode: () => ({}),
    matches: () => false,
    getBoundingClientRect: (): Box => ({
      x,
      y: 0,
      width: 20,
      height: 20,
      left: x,
      right: x + 20,
      top: 0,
      bottom: 20,
    }),
  }
  return {
    element,
    /** Paints the next frame with the button at `position`. */
    async paint(position: number): Promise<void> {
      const callback = frame
      if (callback === undefined) {
        throw new Error('the page function asked for no frame')
      }
      frame = undefined
      x = position
      callback()
      await Promise.resolve()
    },
    waiting: () => frame !== undefined,
  }
}

test('a button that turns back between two frames is not taken as stopped', async () => {
  const state = await elementState()
  const button = animated()
  const result = state.call(button.element, ['stable'], false, 'probe_test', false)
  // An alternating animation seen by Chrome: the frames just before and just
  // after the turn land on the same position.
  const positions = [154.65625, 182.328125, 210.15625, 210.15625, 182.328125, 154.65625, 126.828125, 99, 71.171875, 43.34375]
  for (const x of positions) {
    if (!button.waiting()) break
    await button.paint(x)
  }
  expect((await result).failed).toBe('stable')
})

test('a button that keeps its box for two frame intervals is stable', async () => {
  const state = await elementState()
  const button = animated()
  const result = state.call(button.element, ['stable'], false, 'probe_test', false)
  for (const x of [10, 10, 10]) {
    await button.paint(x)
  }
  expect((await result).failed).toBeNull()
  expect(button.waiting()).toBe(false)
})
