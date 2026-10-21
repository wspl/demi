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
  point?: [number, number] | null,
  intercept?: boolean,
) => Promise<{ failed: string | null }>

// Composed as `element.rs` composes it, with the shared role resolution in
// its scope.
async function elementState(): Promise<ElementState> {
  const read = (name: string) => readFile(new URL(name, import.meta.url), 'utf8')
  const [aria, state] = await Promise.all([read('./aria-role.js'), read('./element-state.js')])
  return new Function(`const aria = (${aria})(); return (${state})`)() as ElementState
}

/** A 20 px button at `x`, and the frame the page function waits for. */
function animated() {
  let x = 0
  let frame: (() => void) | undefined
  const view = {
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
    hasAttribute: () => false,
    getRootNode: () => ({}),
    matches: () => false,
    closest: () => null,
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

test('a button whose box changes from one frame to the next is not stable', async () => {
  const state = await elementState()
  const button = animated()
  const result = state.call(button.element, ['stable'], false, 'probe_test', false)
  await button.paint(10)
  await button.paint(12)
  expect((await result).failed).toBe('stable')
  // A moving button fails at once; the action checks again after a pause.
  expect(button.waiting()).toBe(false)
})

test('a button that keeps its box from one frame to the next is stable', async () => {
  const state = await elementState()
  const button = animated()
  const result = state.call(button.element, ['stable'], false, 'probe_test', false)
  for (const x of [10, 10]) {
    await button.paint(x)
  }
  expect((await result).failed).toBeNull()
  expect(button.waiting()).toBe(false)
})

test('an interceptor whose first event lands on its element leaves with its callback', async () => {
  const state = await elementState()
  const listeners = new Map<string, (event: object) => void>()
  const view = {
    innerWidth: 1280,
    innerHeight: 720,
    getComputedStyle: () => ({ visibility: 'visible' }),
    addEventListener: (type: string, listener: (event: object) => void) => listeners.set(type, listener),
    removeEventListener: (type: string) => listeners.delete(type),
  }
  const element: Record<string, unknown> = {
    ownerDocument: { defaultView: view },
    localName: 'button',
    isConnected: true,
    getAttribute: () => null,
    hasAttribute: () => false,
    matches: () => false,
    closest: () => null,
    getBoundingClientRect: () => ({ x: 0, y: 0, width: 20, height: 20, left: 0, right: 20, top: 0, bottom: 20 }),
  }
  const document = { nodeType: 9, elementsFromPoint: () => [element], elementFromPoint: () => element }
  element.getRootNode = () => document
  const result = await state.call(element, ['hit'], false, 'probe_test', false, null, true)
  expect(result.failed).toBeNull()
  listeners.get('mousedown')?.({ isTrusted: true, clientX: 10, clientY: 10 })
  expect(listeners.size).toBe(0)
  expect('probe_test' in element).toBe(false)
})
