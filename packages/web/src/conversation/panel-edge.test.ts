import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// A back swipe from the screen's left edge over the full-screen work panel
// belongs to the swipe from its first touch: when a live browser tab fills the
// panel, the remote page sees no press, no move and no lift of it, and the
// view's keyboard field takes no focus. The panel is the product's frame
// (SidebarLayout) holding the browser plugin's live view over a live session
// whose stream the test plays, on a page of happy-dom 390 px wide; the finger
// sends what Chrome sends for a touch, a pointer event before each touch event.
// A few ms, besides the start of the page every happy-dom test pays.

GlobalRegistrator.register({ url: 'https://demi.test/', width: 390, height: 844 })
await import('@demicodes/web-ui/__tests__/vue-components')
const { createApp, defineComponent, h, nextTick, ref } = await import('vue')
const { default: SidebarLayout } = await import('@demicodes/web-ui/sidebar/SidebarLayout.vue')
const { createOverlayStore } = await import('@demicodes/web-ui/overlay/overlayStore')
const { definePage, pageContext, providePage } = await import('@demicodes/web-ui/plugins/page')
const { default: LiveView } = await import('@demicodes/plugin-browser/live/LiveView.vue')
const { LiveSession } = await import('@demicodes/plugin-browser/live/session')
type LiveTab = import('@demicodes/plugin-browser/generated/plugin').LiveTab
type LiveModuleMessage = import('@demicodes/plugin-browser/generated/plugin').LiveModuleMessage
type LiveViewerMessage = import('@demicodes/plugin-browser/generated/plugin').LiveViewerMessage
type PageHost = import('@demicodes/web-ui/plugins/page').PageHost
type UserStreamHandlers = import('@demicodes/plugin-sdk').UserStreamHandlers

let unmount: (() => void) | null = null
afterEach(() => {
  unmount?.()
  unmount = null
  document.body.innerHTML = ''
})

const TAB: LiveTab = {
  id: 't1',
  title: 'Orders',
  url: 'https://example.test/',
  createdBy: { kind: 'agent', number: 0 },
  viewport: { width: 390, height: 800, devicePixelRatio: 2, mode: 'mobile' },
  loading: false,
  canGoBack: false,
  canGoForward: false,
}

/** A module's control message, framed as the Host writes it. */
function moduleFrame(message: LiveModuleMessage): Uint8Array {
  const payload = new TextEncoder().encode(JSON.stringify(message))
  const frame = new Uint8Array(5 + payload.length)
  new DataView(frame.buffer).setUint32(0, 1 + payload.length)
  frame[4] = 1
  frame.set(payload, 5)
  return frame
}

/** A host of which the live view uses only its overlays; the rest refuses to be used. */
function host(): PageHost {
  const unused = () => {
    throw new Error('the test does not use this part of the host')
  }
  return {
    userState: unused,
    followState: unused,
    call: unused,
    stream: unused,
    installed: unused,
    hostStarting: unused,
    files: unused,
    intents: { open: unused, canOpen: unused },
    panel: { tabs: unused, select: unused },
    openSettings: unused,
    openLink: unused,
    overlays: createOverlayStore(),
  }
}

/** The panel open full screen with the live tab in it; `sent` is what the view sent the Host. */
async function openLiveTab() {
  const sent: LiveViewerMessage[] = []
  const decoder = new TextDecoder()
  let stream: UserStreamHandlers | null = null
  const session = new LiveSession({
    open: (handlers: UserStreamHandlers) => {
      stream = handlers
      return {
        send: (bytes) => {
          if (bytes[4] === 1)
            sent.push(JSON.parse(decoder.decode(bytes.subarray(5))) as LiveViewerMessage)
        },
        close: () => stream?.closed('closed'),
      }
    },
    platform: 'mac',
    panel: () => null,
    now: () => 0,
    defect: () => {},
  })
  session.start()
  stream!.data(moduleFrame({ type: 'state', running: true, list: 1, tabs: [TAB], watched: TAB.id }))
  const asideOpen = ref(true)
  const page = definePage({ plugin: 'browser' })
  const Tab = defineComponent(() => {
    providePage(pageContext(host(), page))
    return () => h(LiveView, { session, tab: TAB, shown: true, panel: { width: 390, height: 800 }, snap: { x: 0, y: 0 } })
  })
  const root = document.createElement('div')
  document.body.append(root)
  const app = createApp(defineComponent(() => () => h(SidebarLayout, {
    asideOpen: asideOpen.value,
    'onUpdate:asideOpen': (value: boolean) => {
      asideOpen.value = value
    },
  }, {
    sidebar: () => h('nav', 'Conversations'),
    default: () => h('div', 'Conversation'),
    aside: () => h(Tab),
  })))
  app.mount(root)
  unmount = () => {
    app.unmount()
    session.close()
  }
  await nextTick()
  const canvas = root.querySelector('canvas')!
  const field = root.querySelector('input')!
  return { asideOpen, sent, canvas, field }
}

/** What a finger at `x` does, as Chrome sends it: the pointer event, then the touch event, at `t` ms. */
function finger(target: Element, phase: 'start' | 'move' | 'end', t: number, x: number): void {
  const pointerType = { start: 'pointerdown', move: 'pointermove', end: 'pointerup' }[phase]
  target.dispatchEvent(new PointerEvent(pointerType, { bubbles: true, cancelable: true, pointerId: 7, pointerType: 'touch', isPrimary: true, clientX: x, clientY: 400, buttons: phase === 'end' ? 0 : 1 }))
  const touches = phase === 'end' ? [] : [new Touch({ identifier: 7, target, clientX: x, clientY: 400 })]
  const event = new TouchEvent(`touch${phase}`, { bubbles: true, cancelable: true, touches, changedTouches: touches })
  Object.defineProperty(event, 'timeStamp', { value: t })
  target.dispatchEvent(event)
}

test('a back swipe from the edge over a live browser tab never reaches the page: no press, no move, no lift, no keyboard', async () => {
  const { asideOpen, sent, canvas, field } = await openLiveTab()
  const before = sent.length
  finger(canvas, 'start', 0, 6)
  for (const [t, x] of [[16, 40], [32, 90], [48, 160]] as const)
    finger(canvas, 'move', t, x)
  finger(canvas, 'end', 64, 160)
  await nextTick()
  expect(asideOpen.value).toBe(false)
  expect(sent.slice(before).filter((message) => message.type === 'input')).toEqual([])
  expect(document.activeElement === field).toBe(false)
})

test('a finger away from the edge still operates the page', async () => {
  const { asideOpen, sent, canvas } = await openLiveTab()
  const before = sent.length
  finger(canvas, 'start', 0, 120)
  finger(canvas, 'end', 40, 120)
  await nextTick()
  expect(asideOpen.value).toBe(true)
  expect(sent.slice(before).flatMap((message) => message.type === 'input' && message.input.type === 'pointer' ? [message.input.action] : [])).toEqual(['down', 'up'])
})
