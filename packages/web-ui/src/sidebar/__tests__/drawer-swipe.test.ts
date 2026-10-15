import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// A finger on the phone's drawer (`web-application.md` § The phone layout):
// a leftward swipe drags the drawer from where the finger first touched, a
// lift past 30% of its width or with a flick still moving closes it from
// where it stands, a pull held still before the lift springs back, and a
// second finger or the system taking the touch springs it back too; a
// finger never reorders the list. The drawer is SidebarLayout on a page of
// happy-dom 390 px wide, touched by touch events whose times the test gives;
// happy-dom lays nothing out, so the drawer is given its 320 px width. A few
// ms, besides the start of the page every happy-dom test pays.

GlobalRegistrator.register({ url: 'https://demi.test/', width: 390, height: 844 })
await import('../../__tests__/vue-components')
const { createApp, defineComponent, h, nextTick, ref } = await import('vue')
const { default: SidebarLayout } = await import('../SidebarLayout.vue')
const { useSidebarDrag } = await import('../useSidebarDrag')

const DRAWER_WIDTH = 320

let unmount: (() => void) | null = null
afterEach(() => {
  unmount?.()
  unmount = null
  document.body.innerHTML = ''
})

/** The drawer open on a phone; `open` is the frame's model, `drawer` the element a finger touches. */
async function openDrawer() {
  const open = ref(true)
  const host = document.createElement('div')
  document.body.append(host)
  const app = createApp(defineComponent(() => () => h(SidebarLayout, {
    open: open.value,
    'onUpdate:open': (value: boolean) => {
      open.value = value
    },
  }, { sidebar: () => h('nav', 'Conversations'), default: () => h('div', 'Conversation') })))
  app.mount(host)
  unmount = () => app.unmount()
  await nextTick()
  const drawer = host.querySelector<HTMLElement>('[data-region]')!
  Object.defineProperty(drawer, 'offsetWidth', { value: DRAWER_WIDTH, configurable: true })
  return { open, drawer }
}

/** A touch event at `t` ms with the fingers at `points`, as Safari sends them. */
function touch(drawer: HTMLElement, type: string, t: number, ...points: [x: number, y: number][]): TouchEvent {
  const touches = points.map(([x, y], index) => new Touch({ identifier: index, target: drawer, clientX: x, clientY: y }))
  const event = new TouchEvent(type, { bubbles: true, cancelable: true, touches, changedTouches: touches })
  Object.defineProperty(event, 'timeStamp', { value: t })
  drawer.dispatchEvent(event)
  return event
}

/** Where the drawer stands now: px left of its place, or null in place, and whether it follows a finger. */
function drawerAt(drawer: HTMLElement) {
  const pull = drawer.style.getPropertyValue('--drawer-pull')
  return { pull: pull === '' ? null : Number.parseFloat(pull), held: drawer.hasAttribute('data-pulled') }
}

/** A finger down at x 300 that moves left to each of `xs`, 16 ms apart from t 0. */
function pull(drawer: HTMLElement, ...xs: number[]): number {
  touch(drawer, 'touchstart', 0, [300, 400])
  let t = 0
  for (const x of xs) {
    t += 16
    touch(drawer, 'touchmove', t, [x, 400])
  }
  return t
}

test('the drawer follows the finger from where it first touched', async () => {
  const { drawer } = await openDrawer()
  pull(drawer, 290, 250, 190)
  await nextTick()
  expect(drawerAt(drawer)).toEqual({ pull: -110, held: true })
})

test('a pull let go past 30% of the width closes the drawer from where it stands', async () => {
  const { open, drawer } = await openDrawer()
  const t = pull(drawer, 290, 250, 200, 180, 170, 170)
  touch(drawer, 'touchend', t + 200)
  await nextTick()
  expect(open.value).toBe(false)
  // It leaves from 130 px, eased, not from its place.
  expect(drawerAt(drawer)).toEqual({ pull: -130, held: false })
})

test('a closing drawer\'s scrim fades from where it stands, eased', async () => {
  const { open, drawer } = await openDrawer()
  const t = pull(drawer, 290, 250, 200, 180, 170, 170)
  // The page draws the pull before the finger lifts.
  await nextTick()
  touch(drawer, 'touchend', t + 200)
  await nextTick()
  expect(open.value).toBe(false)
  // Still on the page while it fades, no longer following a finger: its fade is eased.
  const scrim = document.querySelector<HTMLElement>('.frame-scrim')
  expect({ shown: scrim !== null, held: scrim?.hasAttribute('data-pulled') }).toEqual({ shown: true, held: false })
})

test('a flick of a single move closes the drawer', async () => {
  const { open, drawer } = await openDrawer()
  touch(drawer, 'touchstart', 0, [300, 400])
  touch(drawer, 'touchmove', 16, [240, 400])
  touch(drawer, 'touchend', 20)
  await nextTick()
  expect(open.value).toBe(false)
})

test('a second finger put down on the scrim springs the drawer back', async () => {
  const { open, drawer } = await openDrawer()
  pull(drawer, 290, 250, 200, 140)
  // The scrim's finger shows only in the list of every finger on the screen.
  touch(drawer, 'touchmove', 80, [130, 400], [370, 600])
  touch(drawer, 'touchend', 100, [370, 600])
  await nextTick()
  expect(open.value).toBe(true)
  expect(drawerAt(drawer)).toEqual({ pull: null, held: false })
})

test('a flick held still before the lift springs back, and one still moving at the lift closes', async () => {
  const held = await openDrawer()
  const flicked = pull(held.drawer, 290, 270, 240)
  touch(held.drawer, 'touchend', flicked + 687)
  await nextTick()
  expect(held.open.value).toBe(true)
  expect(drawerAt(held.drawer)).toEqual({ pull: null, held: false })
  unmount?.()

  const moving = await openDrawer()
  const lifted = pull(moving.drawer, 290, 270, 240)
  touch(moving.drawer, 'touchend', lifted + 4)
  await nextTick()
  expect(moving.open.value).toBe(false)
})

test('a second finger or the system taking the touch springs the drawer back, and only a lift closes it', async () => {
  const fingers = await openDrawer()
  pull(fingers.drawer, 290, 250, 200, 140)
  touch(fingers.drawer, 'touchstart', 80, [140, 400], [200, 500])
  touch(fingers.drawer, 'touchend', 100, [200, 500])
  await nextTick()
  expect(fingers.open.value).toBe(true)
  expect(drawerAt(fingers.drawer)).toEqual({ pull: null, held: false })
  unmount?.()

  const cancelled = await openDrawer()
  const t = pull(cancelled.drawer, 290, 250, 200, 140)
  touch(cancelled.drawer, 'touchcancel', t + 4)
  await nextTick()
  expect(cancelled.open.value).toBe(true)
  expect(drawerAt(cancelled.drawer)).toEqual({ pull: null, held: false })
})

test('a finger dragged along a row never reorders the list, while a mouse does', async () => {
  const container = document.createElement('div')
  const rows = ['a', 'b'].map((id) => {
    const row = document.createElement('div')
    row.dataset.sidebarId = id
    row.dataset.sidebarKind = 'conversation'
    container.append(row)
    return row
  })
  document.body.append(container)
  let drag: ReturnType<typeof useSidebarDrag> | undefined
  const app = createApp(defineComponent(() => {
    drag = useSidebarDrag(ref(container), () => [], () => [], () => {})
    return () => null
  }))
  app.mount(document.createElement('div'))
  unmount = () => app.unmount()
  const press = (pointerType: string) => {
    const down = new PointerEvent('pointerdown', { pointerId: 1, pointerType, button: 0, clientX: 100, clientY: 20 })
    rows[0]!.dispatchEvent(down)
    drag!.start(down, { kind: 'conversation', id: 'a' })
    // Diagonally down the list, as a swipe that starts to scroll does.
    window.dispatchEvent(new PointerEvent('pointermove', { pointerId: 1, pointerType, clientX: 60, clientY: 60 }))
  }
  press('touch')
  expect(drag!.source.value).toBeNull()
  window.dispatchEvent(new PointerEvent('pointerup', { pointerId: 1, pointerType: 'touch' }))
  press('mouse')
  expect(drag!.source.value).toEqual({ kind: 'conversation', id: 'a' })
})
