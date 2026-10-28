// The page the watch scene's browser shows: Paperwell, an invented
// office-supply shop (`PaperwellPage.vue`), where the agent's pointer adds
// the usual order to the cart and opens the cart, then stops before payment.
// Drawn over the live view (`cover.ts`) in a tab of 400 × 520 CSS pixels,
// the work panel's width in the demo's frame; the live view itself shows a
// blank page under it.
import type { GalleryPage } from '@demicodes/web-gallery/fixtures/live-browser'
import { cover, type Cover } from './cover'
import PaperwellPage from './PaperwellPage.vue'

interface Point {
  x: number
  y: number
}

const WIDTH = 400
const HEIGHT = 520

/** Each product's Add to Cart button, two products to a row, where the shop's picture has it. */
const BUTTONS: Point[] = [{ x: 105, y: 286 }, { x: 294, y: 286 }, { x: 105, y: 480 }, { x: 294, y: 480 }]
const BUTTON = { width: 81, height: 26 }
/** The cart's icon in the page's header, which the pointer opens and the count sits on. */
const CART = { x: 369, y: 23 }

function buttonCenter(index: number): Point {
  return { x: BUTTONS[index]!.x + BUTTON.width / 2, y: BUTTONS[index]!.y + BUTTON.height / 2 }
}

/** Where the pointer goes and when: each stop's time in seconds, and whether it clicks there. */
const PATH: { at: number; to: Point; click?: number }[] = [
  { at: 0, to: { x: 220, y: 200 } },
  { at: 0.6, to: buttonCenter(0), click: 0 },
  { at: 1.1, to: buttonCenter(1), click: 1 },
  { at: 1.6, to: buttonCenter(2), click: 2 },
  { at: 2.1, to: buttonCenter(3), click: 3 },
  { at: 2.6, to: CART },
  { at: 3.3, to: { x: 290, y: 400 } },
]
/** When the cart opens, after the pointer reached it. */
const CART_AT = 2.75
/** How long a pressed button stays dark. */
const PRESS_S = 0.18
/** How long the pointer rests at a stop before it moves on, and arrives before the next one's time. */
const REST_S = 0.1
const ARRIVE_S = 0.15
/** How long the pointer stays once it has stopped for good. */
const SHOWN_S = 3.6
const SVG = 'http://www.w3.org/2000/svg'

/** The agent's pointer: its tip at the element's top left corner. */
function pointer(): SVGSVGElement {
  const svg = document.createElementNS(SVG, 'svg')
  svg.setAttribute('viewBox', '-1 -1 15 22')
  svg.setAttribute('width', '15')
  svg.setAttribute('height', '22')
  svg.style.cssText = 'position: absolute; left: -1px; top: -1px; overflow: visible;'
  const path = document.createElementNS(SVG, 'path')
  path.setAttribute('d', 'M0 0 L0 17 L4.5 13 L7.5 19.5 L10.5 18.2 L7.6 11.8 L13 11.8 Z')
  path.setAttribute('fill', '#111')
  path.setAttribute('stroke', '#fff')
  path.setAttribute('stroke-width', '1.5')
  path.setAttribute('stroke-linejoin', 'round')
  path.setAttribute('paint-order', 'stroke')
  svg.append(path)
  return svg
}

interface Shop {
  count: number
  pressed: number | null
  cart: boolean
}

/** Draws the shop over `canvas`, the live view of its tab, as it first shows. */
export function showShop(canvas: HTMLCanvasElement): Cover<Shop> {
  return cover<Shop>(canvas, PaperwellPage, { count: 0, pressed: null, cart: false }, { width: WIDTH, height: HEIGHT })
}

/**
 * Starts the page's clock and the pointer's glide over it: the pointer
 * rests a moment at each stop, then eases to the next, and the page answers
 * each click as it lands.
 */
export function begin({ props, content }: Cover<Shop>): void {
  const hand = document.createElement('div')
  hand.style.cssText = 'position: absolute; left: 0; top: 0;'
  hand.append(pointer())
  content.append(hand)
  const at = (point: Point): string => `translate(${point.x}px, ${point.y}px)`
  const keyframes: Keyframe[] = [{ offset: 0, transform: at(PATH[0]!.to), easing: 'linear' }]
  for (const [index, stop] of PATH.entries()) {
    if (index === 0)
      continue
    const previous = PATH[index - 1]!
    keyframes.push(
      { offset: (previous.at + REST_S) / SHOWN_S, transform: at(previous.to), easing: 'cubic-bezier(0.45, 0, 0.55, 1)' },
      { offset: Math.max(previous.at + REST_S, stop.at - ARRIVE_S) / SHOWN_S, transform: at(stop.to), easing: 'linear' },
    )
  }
  keyframes.push({ offset: 1, transform: at(PATH.at(-1)!.to) })
  hand.animate(keyframes, { duration: SHOWN_S * 1000, fill: 'both' })
  setTimeout(() => hand.remove(), SHOWN_S * 1000)
  let clicks = 0
  for (const stop of PATH) {
    if (stop.click === undefined)
      continue
    setTimeout(() => {
      clicks += 1
      props.count = clicks
      props.pressed = stop.click!
    }, stop.at * 1000)
    setTimeout(() => {
      if (props.pressed === stop.click)
        props.pressed = null
    }, (stop.at + PRESS_S) * 1000)
  }
  setTimeout(() => (props.cart = true), CART_AT * 1000)
}

/** The live view's own page: blank, under the shop drawn over it. */
export const SHOP: GalleryPage = {
  width: WIDTH,
  height: HEIGHT,
  paint: (context) => {
    context.fillStyle = '#ffffff'
    context.fillRect(0, 0, WIDTH, HEIGHT)
  },
}
