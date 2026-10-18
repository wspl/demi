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
const { installPhonePage } = await import('../../ui/phone-page')

const DRAWER_WIDTH = 320

let unmount: (() => void) | null = null
afterEach(() => {
  unmount?.()
  unmount = null
  document.body.innerHTML = ''
})

/** The drawer open on a phone; `open` is the frame's model, `drawer` the element a finger touches. */
async function openDrawer(isOpen = true) {
  const open = ref(isOpen)
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
  return { open, drawer, page: host.querySelector<HTMLElement>('main')! }
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

// From the screen's left edge a rightward swipe opens the closed drawer
// (`web-application.md` § The phone layout, "The edges are the app's"),
// following the finger and settling by the closing swipe's rule.

/** A finger down on the conversation at x 4, the screen's edge, that moves to each of `points`, 16 ms apart from t 0. */
function drawIn(page: HTMLElement, ...points: [x: number, y: number][]): number {
  touch(page, 'touchstart', 0, [4, 400])
  let t = 0
  for (const point of points) {
    t += 16
    touch(page, 'touchmove', t, point)
  }
  return t
}

/** Whether the drawer is on the screen, which a closed one is not. */
function shown(drawer: HTMLElement): boolean {
  return drawer.style.display !== 'none'
}

test('a swipe right from the left edge draws the drawer in after the finger, with the scrim, and a lift past 30% opens it', async () => {
  const { open, drawer, page } = await openDrawer(false)
  expect(shown(drawer)).toBe(false)
  const t = drawIn(page, [14, 400], [60, 402], [140, 404])
  await nextTick()
  // Its right edge is 136 px out, where the finger has moved it from its first touch.
  expect({ shown: shown(drawer), ...drawerAt(drawer) }).toEqual({ shown: true, pull: -184, held: true })
  expect(document.querySelector<HTMLElement>('.frame-scrim')?.style.getPropertyValue('--scrim-shade')).toBe(String(1 - 184 / DRAWER_WIDTH))
  touch(page, 'touchend', t + 200)
  await nextTick()
  expect(open.value).toBe(true)
  // It goes on into its place, eased, from where the finger left it.
  expect(drawerAt(drawer)).toEqual({ pull: null, held: false })
})

test('a short pull from the edge slides the drawer back out, and a flick of one opens it', async () => {
  const short = await openDrawer(false)
  const t = drawIn(short.page, [20, 400], [60, 400])
  touch(short.page, 'touchend', t + 200)
  await nextTick()
  expect(short.open.value).toBe(false)
  expect(shown(short.drawer)).toBe(false)
  unmount?.()

  const flicked = await openDrawer(false)
  touch(flicked.page, 'touchstart', 0, [4, 400])
  touch(flicked.page, 'touchmove', 16, [64, 400])
  touch(flicked.page, 'touchend', 20)
  await nextTick()
  expect(flicked.open.value).toBe(true)
})

test('a move down from the edge, or a swipe right away from it, draws no drawer', async () => {
  const { open, drawer, page } = await openDrawer(false)
  const down = drawIn(page, [6, 420], [10, 500])
  touch(page, 'touchmove', down + 16, [200, 500])
  touch(page, 'touchend', down + 20)
  await nextTick()
  expect([open.value, shown(drawer)]).toEqual([false, false])

  touch(page, 'touchstart', 0, [40, 400])
  touch(page, 'touchmove', 16, [200, 400])
  touch(page, 'touchend', 20)
  await nextTick()
  expect([open.value, shown(drawer)]).toEqual([false, false])
})

// Over the full-screen work panel the left edge is the panel's: a rightward
// swipe from it takes the panel back to the conversation beneath, following
// the finger and settling by the drawer's rule (`web-application.md` § The
// phone layout, "Whole-screen panels"), and draws no drawer.

/** The work panel open full screen on a phone; `panel` is the element a finger touches. */
async function openPanel(content: () => ReturnType<typeof h> = () => h('aside', { class: 'panel' }, 'Work panel')) {
  const open = ref(false)
  const asideOpen = ref(true)
  const host = document.createElement('div')
  document.body.append(host)
  const app = createApp(defineComponent(() => () => h(SidebarLayout, {
    open: open.value,
    'onUpdate:open': (value: boolean) => {
      open.value = value
    },
    asideOpen: asideOpen.value,
    'onUpdate:asideOpen': (value: boolean) => {
      asideOpen.value = value
    },
  }, {
    sidebar: () => h('nav', 'Conversations'),
    default: () => h('div', 'Conversation'),
    aside: content,
  })))
  app.mount(host)
  unmount = () => app.unmount()
  await nextTick()
  const drawer = host.querySelector<HTMLElement>('[data-region]')!
  const panel = host.querySelector<HTMLElement>('aside.panel')!
  const frame = host.querySelector<HTMLElement>('.frame-aside')!
  return { open, asideOpen, drawer, panel, frame }
}

/** Where the panel stands now: px right of its place, or null in place, and whether it follows a finger. */
function panelAt(frame: HTMLElement) {
  const pull = frame.style.getPropertyValue('--aside-pull')
  return { pull: pull === '' ? null : Number.parseFloat(pull), held: frame.hasAttribute('data-pulled') }
}

test('a swipe right from the left edge drags the panel after the finger, and a lift past 30% of the width takes it back', async () => {
  const { asideOpen, open, drawer, panel, frame } = await openPanel()
  const t = drawIn(panel, [14, 400], [60, 402], [130, 404])
  await nextTick()
  expect(panelAt(frame)).toEqual({ pull: 126, held: true })
  touch(panel, 'touchend', t + 200)
  await nextTick()
  expect(asideOpen.value).toBe(false)
  // It goes on sliding away from where the finger left it, eased; the drawer stayed closed.
  expect(panelAt(frame)).toEqual({ pull: 126, held: false })
  expect([open.value, shown(drawer)]).toEqual([false, false])
})

test('a short or held pull on the panel springs it back, and a flick of one takes it back', async () => {
  const short = await openPanel()
  const t = drawIn(short.panel, [20, 400], [100, 400])
  // 96 px is short of 30% of the 390 px screen, and the finger stood still before the lift.
  touch(short.panel, 'touchend', t + 200)
  await nextTick()
  expect(short.asideOpen.value).toBe(true)
  expect(panelAt(short.frame)).toEqual({ pull: null, held: false })
  unmount?.()

  const flicked = await openPanel()
  touch(flicked.panel, 'touchstart', 0, [4, 400])
  touch(flicked.panel, 'touchmove', 16, [64, 400])
  touch(flicked.panel, 'touchend', 20)
  await nextTick()
  expect(flicked.asideOpen.value).toBe(false)
})

test('over the panel the edge is the panel\'s swipe: the drawer neither shows nor opens, and a swipe away from the edge drags nothing', async () => {
  const { asideOpen, open, drawer, panel, frame } = await openPanel()
  const t = drawIn(panel, [14, 400], [60, 400], [200, 400])
  await nextTick()
  expect([shown(drawer), drawer.hasAttribute('data-pulled')]).toEqual([false, false])
  touch(panel, 'touchend', t + 4)
  await nextTick()
  expect([asideOpen.value, open.value]).toEqual([false, false])
  unmount?.()

  const away = await openPanel()
  touch(away.panel, 'touchstart', 0, [40, 400])
  touch(away.panel, 'touchmove', 16, [200, 400])
  touch(away.panel, 'touchend', 20)
  await nextTick()
  expect([away.asideOpen.value, panelAt(away.frame)]).toEqual([true, { pull: null, held: false }])
})

test('a short flick from the edge takes the panel back and clicks nothing under the finger', async () => {
  // The phone's page gives an edge touch that lifts within 10 px its click (`phone-page.ts`); the drag takes it at 8.
  const stopPage = installPhonePage()
  try {
    let clicks = 0
    const { asideOpen, panel } = await openPanel(() => h('aside', { class: 'panel' }, [
      h('button', { onClick: () => clicks++ }, 'Under the finger'),
    ]))
    const button = panel.querySelector('button')!
    touch(button, 'touchstart', 0, [4, 400])
    touch(button, 'touchmove', 10, [13, 400])
    touch(button, 'touchend', 12)
    await nextTick()
    expect([asideOpen.value, clicks]).toEqual([false, 0])
  } finally {
    stopPage()
  }
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
