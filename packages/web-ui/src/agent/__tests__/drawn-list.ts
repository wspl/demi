import { createApp, defineComponent, nextTick, type VNode } from 'vue'
import { provideMediaViewer } from '../../files/media-viewer'
import { provideBlobUrl } from '../media-source'

/**
 * Draws `render`, a transcript list or a window that holds one, on a
 * happy-dom page, whose rows measure 40 px and every other element 2000 px,
 * so the list's virtualizer draws its rows: happy-dom lays nothing out. The
 * page registers happy-dom before it imports this. Unmount restores the
 * measures.
 */
export function drawn(render: () => VNode): { host: HTMLElement, unmount: () => void } {
  const rowHigh = (el: Element) => (el.hasAttribute('data-index') ? 40 : 2000)
  const real = {
    rect: Element.prototype.getBoundingClientRect,
    client: Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'clientHeight'),
    offset: Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'offsetHeight'),
  }
  Element.prototype.getBoundingClientRect = function () {
    return DOMRect.fromRect({ x: 0, y: 0, width: 800, height: rowHigh(this) })
  }
  Object.defineProperty(HTMLElement.prototype, 'clientHeight', { configurable: true, get() { return rowHigh(this) } })
  Object.defineProperty(HTMLElement.prototype, 'offsetHeight', { configurable: true, get() { return rowHigh(this) } })
  const host = document.createElement('div')
  document.body.append(host)
  const app = createApp(defineComponent({
    setup: () => {
      provideBlobUrl((source) => `blob:${source}`)
      provideMediaViewer()
      return render
    },
  }))
  app.mount(host)
  return {
    host,
    unmount: () => {
      app.unmount()
      host.remove()
      Element.prototype.getBoundingClientRect = real.rect
      for (const [name, descriptor] of [['clientHeight', real.client], ['offsetHeight', real.offset]] as const) {
        if (descriptor) {
          Object.defineProperty(HTMLElement.prototype, name, descriptor)
        }
      }
    },
  }
}

const frame = () => new Promise<void>((done) => requestAnimationFrame(() => done()))

/** Lets the page draw what changed: a few frames, each with Vue's updates. */
export async function settled(): Promise<void> {
  for (let more = 0; more < 5; more += 1) {
    await frame()
    await nextTick()
  }
}

/** What the transcript's tail row says, Requesting or Connecting with its time; null while none shows. */
export function tailRow(host: HTMLElement): string | null {
  // The row's words roll as they change: the first face is the one shown.
  return host.querySelector('[data-transcript-wait] .truncate')?.textContent?.trim() ?? null
}
