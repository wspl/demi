// A screen the demo draws itself over a live view of the work panel: the
// page in the browser, the app in the simulator. The live view is a canvas,
// as a Host's stream is, and a recording would keep it as a picture that
// blurs as the camera comes close; drawn as the page's own elements, the
// screen stays sharp, and the recording keeps it as it keeps the rest.
import { createApp, h, reactive, type Component } from 'vue'

export interface Cover<P extends object> {
  /** The screen's props: a change shows at once. */
  props: P
  /** The screen's box, in the screen's own pixels, for what moves over it. */
  content: HTMLElement
}

/**
 * Covers `canvas` with `screen`, laid out in `size` of its own pixels and
 * scaled to the canvas, its corners rounded by `radius` of them; the cover
 * follows the canvas as it moves and resizes, a panel sliding in included.
 */
export function cover<P extends object>(canvas: HTMLCanvasElement, screen: Component, initial: P, size: { width: number; height: number }, radius = 0): Cover<P> {
  const props = reactive(initial) as P
  const layer = document.createElement('div')
  layer.style.cssText = 'position: fixed; overflow: hidden; pointer-events: none; z-index: 40;'
  const content = document.createElement('div')
  content.style.cssText = `position: absolute; left: 0; top: 0; width: ${size.width}px; height: ${size.height}px; transform-origin: 0 0;`
  layer.append(content)
  const mount = document.createElement('div')
  content.append(mount)
  document.body.append(layer)
  createApp({ render: () => h(screen, props) }).mount(mount)
  let shown = ''
  const follow = (): void => {
    if (!canvas.isConnected) {
      layer.remove()
      return
    }
    const box = canvas.getBoundingClientRect()
    const scale = box.width / size.width
    const place = `${box.left} ${box.top} ${box.width} ${box.height}`
    if (place !== shown) {
      shown = place
      Object.assign(layer.style, { left: `${box.left}px`, top: `${box.top}px`, width: `${box.width}px`, height: `${box.height}px`, borderRadius: `${radius * scale}px` })
      content.style.transform = `scale(${scale})`
    }
    requestAnimationFrame(follow)
  }
  follow()
  return { props, content }
}
