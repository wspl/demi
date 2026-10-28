// The app the phone scene's simulator shows: Harbor, an invented app whose
// sign-up the agent tests (`HarborScreen.vue`). It moves on as the scene's
// commands do: a field fills as the agent types into it, the button darkens
// under its tap, then the welcome screen shows. Drawn over the live view
// (`cover.ts`), in the simulator's points; the live view itself shows the
// screen's blank background under it.
import type { GalleryScreen } from '@demicodes/web-gallery/fixtures/live-mobile'
import { cover, type Cover } from './cover'
import HarborScreen from './HarborScreen.vue'

/** What the screen shows, in the order the scene goes through it. */
type Moment = 'empty' | 'email' | 'password' | 'tapped' | 'welcome'

const EMAIL = 'alex@example.com'
const PASSWORD_LENGTH = 10
/** How long the agent takes to type a field's text, in seconds. */
const TYPING_S = 0.6
/** How long a tap's ring shows. */
const TAP_S = 0.4
/** The Work iPhone's screen, in points, and its corners' radius. */
const POINTS = { width: 402, height: 874 }
const CORNER = 62
const BACKGROUND = '#f6f8f8'

interface Harbor {
  welcome: boolean
  email: string
  dots: number
  typing: 'email' | 'password' | null
  pressed: boolean
  ring: boolean
}

let moment: Moment = 'empty'
/** When the screen reached `moment`. */
let since = 0
let screen: Cover<Harbor> | null = null

function typed(length: number, seconds: number): number {
  return Math.round(length * Math.min(1, seconds / TYPING_S))
}

/** What the screen shows now, as `moment` has gone on for a while. */
function update(): void {
  if (!screen)
    return
  const seconds = (performance.now() - since) / 1000
  const shown: Harbor = {
    welcome: moment === 'welcome',
    email: moment === 'empty' ? '' : moment === 'email' ? EMAIL.slice(0, typed(EMAIL.length, seconds)) : EMAIL,
    dots: moment === 'empty' || moment === 'email' ? 0 : moment === 'password' ? typed(PASSWORD_LENGTH, seconds) : PASSWORD_LENGTH,
    typing: moment === 'email' || moment === 'password' ? moment : null,
    pressed: moment === 'tapped',
    ring: moment === 'tapped' && seconds < TAP_S,
  }
  for (const key of Object.keys(shown) as (keyof Harbor)[]) {
    if (screen.props[key] !== shown[key])
      (screen.props as Record<keyof Harbor, unknown>)[key] = shown[key]
  }
  requestAnimationFrame(update)
}

/** The scene's commands reach `next`: the screen moves on from now. */
export function harborShows(next: Moment): void {
  moment = next
  since = performance.now()
}

/** Draws Harbor over `canvas`, the live view of the simulator's screen. */
export function showHarbor(canvas: HTMLCanvasElement): void {
  screen = cover<Harbor>(canvas, HarborScreen, { welcome: false, email: '', dots: 0, typing: null, pressed: false, ring: false }, POINTS, CORNER)
  update()
}

/** The live view's own screen: Harbor's background, under the app drawn over it. */
export const HARBOR: GalleryScreen = {
  paint(context, { width, height }) {
    context.fillStyle = BACKGROUND
    context.fillRect(0, 0, width, height)
  },
}
