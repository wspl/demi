import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// A finger pulling a phone's sheet down by its grabber: the sheet follows the
// finger from where it first touched, and a second finger or the system taking
// the touch springs it back, since only a lift closes it. The sheet is
// Sheet.vue on a page of happy-dom, touched by touch events; happy-dom lays
// nothing out, so the sheet is given its 400 px height. A few ms, besides the
// start of the page every happy-dom test pays.

GlobalRegistrator.register({ url: 'https://demi.test/', width: 390, height: 844 })
await import('../../__tests__/vue-components')
const { createApp, defineComponent, h, nextTick, ref } = await import('vue')
const { default: Sheet } = await import('../Sheet.vue')

let unmount: (() => void) | null = null
afterEach(() => {
  unmount?.()
  unmount = null
  document.body.innerHTML = ''
})

/** An open sheet whose owner closes it when it asks; `closed` counts its asks, `grabber` is where a finger takes it. */
async function openSheet() {
  const closed = ref(0)
  const open = ref(true)
  const app = createApp(defineComponent(() => () => h(Sheet, {
    open: open.value,
    label: 'Conversation',
    onClose: () => {
      closed.value++
      open.value = false
    },
  }, { default: () => h('p', 'Rename') })))
  app.mount(document.createElement('div'))
  unmount = () => app.unmount()
  await nextTick()
  const panel = document.querySelector<HTMLElement>('[role="dialog"]')!
  Object.defineProperty(panel, 'offsetHeight', { value: 400, configurable: true })
  return { closed, panel, grabber: panel.querySelector<HTMLElement>('[data-sheet-handle]')! }
}

/** A touch event at `t` ms with the fingers at the heights `ys`, as Safari sends them. */
function touch(target: HTMLElement, type: string, t: number, ...ys: number[]): void {
  const touches = ys.map((y, index) => new Touch({ identifier: index, target, clientX: 200, clientY: y }))
  const event = new TouchEvent(type, { bubbles: true, cancelable: true, touches, changedTouches: touches })
  Object.defineProperty(event, 'timeStamp', { value: t })
  target.dispatchEvent(event)
}

/** A finger down on the grabber at y 450 that moves down to each of `ys`, 16 ms apart from t 0. */
function pull(grabber: HTMLElement, ...ys: number[]): number {
  touch(grabber, 'touchstart', 0, 450)
  let t = 0
  for (const y of ys) {
    t += 16
    touch(grabber, 'touchmove', t, y)
  }
  return t
}

/** How far the sheet stands below its place, in px; null in place. */
function offset(panel: HTMLElement): number | null {
  const value = panel.style.getPropertyValue('--sheet-offset')
  return value === '' ? null : Number.parseFloat(value)
}

test('the sheet follows the finger from where it first touched', async () => {
  const { panel, grabber } = await openSheet()
  pull(grabber, 460, 500, 560)
  await nextTick()
  expect(offset(panel)).toBe(110)
})

test('a sheet pulled down and let go eases down from where it stands', async () => {
  const { closed, panel, grabber } = await openSheet()
  const t = pull(grabber, 460, 520, 600, 620, 620)
  // The page draws the pull before the finger lifts.
  await nextTick()
  touch(grabber, 'touchend', t + 200)
  await nextTick()
  expect(closed.value).toBe(1)
  // Still on the page while it leaves: no longer following a finger, so its fall is eased, from 170 px down.
  expect({ dragging: panel.classList.contains('sheet-dragging'), offset: offset(panel), shown: panel.isConnected })
    .toEqual({ dragging: false, offset: 170, shown: true })
})

test('a second finger put down on the scrim springs the sheet back', async () => {
  const { closed, panel, grabber } = await openSheet()
  pull(grabber, 460, 520, 600)
  touch(grabber, 'touchmove', 64, 610, 200)
  touch(grabber, 'touchend', 80, 200)
  await nextTick()
  expect({ closed: closed.value, offset: offset(panel) }).toEqual({ closed: 0, offset: null })
})

test('a second finger or the system taking the touch springs the sheet back, and only a lift closes it', async () => {
  const fingers = await openSheet()
  pull(fingers.grabber, 460, 520, 600)
  touch(fingers.grabber, 'touchstart', 64, 600, 300)
  touch(fingers.grabber, 'touchend', 80, 300)
  await nextTick()
  expect({ closed: fingers.closed.value, offset: offset(fingers.panel) }).toEqual({ closed: 0, offset: null })
  unmount?.()
  document.body.innerHTML = ''

  const cancelled = await openSheet()
  const t = pull(cancelled.grabber, 460, 520, 600)
  touch(cancelled.grabber, 'touchcancel', t + 4)
  await nextTick()
  expect({ closed: cancelled.closed.value, offset: offset(cancelled.panel) }).toEqual({ closed: 0, offset: null })
})
