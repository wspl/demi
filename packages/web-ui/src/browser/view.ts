/**
 * Where a watched tab's picture sits in the panel, and what the viewer's
 * pointer means in the tab (`browser-live-view.md` § Modes). A Web tab is the
 * panel's own size; Mobile and Custom keep theirs, scaled to fit and centred.
 */
import type { LiveTab, LiveViewport } from '@demicodes/browser-protocol/live'

export interface PanelSize {
  width: number
  height: number
}

export interface Placement {
  /** Panel CSS pixels per tab CSS pixel. */
  scale: number
  left: number
  top: number
  width: number
  height: number
}

/** The largest size the protocol and the Host's capture take. */
const MAX_SIDE = 4096

/** The panel the module hears about: whole CSS pixels within the protocol's range. */
export function panelSize(width: number, height: number): PanelSize {
  const side = (length: number) => Math.max(1, Math.min(MAX_SIDE, Math.round(length)))
  return { width: side(width), height: side(height) }
}

/** The picture at its own size when it fits, scaled down when it does not. */
export function placePicture(viewport: LiveViewport, panel: PanelSize): Placement {
  const scale = Math.min(1, panel.width / viewport.width, panel.height / viewport.height)
  const width = viewport.width * scale
  const height = viewport.height * scale
  return {
    scale,
    left: Math.max(0, (panel.width - width) / 2),
    top: Math.max(0, (panel.height - height) / 2),
    width,
    height,
  }
}

/** The tab's CSS coordinates under a point of the panel. */
export function tabPoint(
  point: { x: number; y: number },
  viewport: LiveViewport,
  placement: Placement,
): { x: number; y: number } {
  const inside = (value: number, length: number) => Math.max(0, Math.min(length, value))
  return {
    x: inside((point.x - placement.left) / placement.scale, viewport.width),
    y: inside((point.y - placement.top) / placement.scale, viewport.height),
  }
}

/** Where a rectangle of the tab lands in the panel, for a control over it. */
export function panelRect(
  rect: { x: number; y: number; width: number; height: number },
  placement: Placement,
): { left: number; top: number; width: number; height: number } {
  return {
    left: placement.left + rect.x * placement.scale,
    top: placement.top + rect.y * placement.scale,
    width: rect.width * placement.scale,
    height: rect.height * placement.scale,
  }
}

/** What the viewport menu offers and shows for a tab. */
export interface ViewportChoice {
  mode: LiveViewport['mode']
  label: string
  /** Only the agent gives a tab its Custom size; the menu cannot choose one. */
  selectable: boolean
}

export function viewportChoices(viewport: LiveViewport): ViewportChoice[] {
  const choices: ViewportChoice[] = [
    { mode: 'web', label: 'Web', selectable: true },
    { mode: 'mobile', label: 'Mobile', selectable: true },
  ]
  if (viewport.mode === 'custom') {
    choices.push({
      mode: 'custom',
      label: `Custom ${viewport.width} × ${viewport.height} @${viewport.devicePixelRatio}`,
      selectable: false,
    })
  }
  return choices
}

/** The tab's address as the bar shows it, and its title for the strip. */
export function tabTitle(tab: LiveTab): string {
  if (tab.title) {
    return tab.title
  }
  const url = URL.parse(tab.url)
  return url ? url.host || url.href : 'New tab'
}
