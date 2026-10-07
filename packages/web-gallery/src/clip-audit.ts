/**
 * Finds outlined elements that a scroll region would clip. A region with
 * `overflow-y: auto` clips the x axis too, so a ring, focus ring, corner badge or
 * a floating layer's shadow on a child too near the region's edge is cut off.
 */
export interface ClipFinding {
  region: Element
  element: Element
  /** How far, in CSS pixels, the outline reaches past the region's padding box. */
  overshoot: number
  side: 'left' | 'right' | 'top' | 'bottom'
}

type Side = ClipFinding['side']

/** How far something painted around a box reaches past each of its edges. */
type Extent = Record<Side, number>

const NO_EXTENT: Extent = { left: 0, right: 0, top: 0, bottom: 0 }

/**
 * How far a box-shadow list reaches past each edge of the box: a shadow's
 * offset moves it toward one side and away from the other, so a shadow that
 * drops below a dialog reaches further down than to the left. A shadow
 * painted in a transparent colour reaches nowhere.
 */
function shadowExtent(boxShadow: string): Extent {
  if (!boxShadow || boxShadow === 'none')
    return NO_EXTENT
  const extent = { ...NO_EXTENT }
  // Colors may contain commas inside rgb(); split on commas outside parentheses.
  for (const shadow of boxShadow.split(/,(?![^(]*\))/)) {
    if (/\binset\b/.test(shadow) || /rgba\([^)]*,\s*0\)/.test(shadow))
      continue
    // The colour's own numbers are not lengths: read the lengths after it.
    const lengths = shadow.replace(/rgba?\([^)]*\)/, '').match(/-?\d*\.?\d+px/g)?.map((v) => parseFloat(v)) ?? []
    const [x = 0, y = 0, blur = 0, spread = 0] = lengths
    const reach = blur + spread
    extent.left = Math.max(extent.left, reach - x)
    extent.right = Math.max(extent.right, reach + x)
    extent.top = Math.max(extent.top, reach - y)
    extent.bottom = Math.max(extent.bottom, reach + y)
  }
  return extent
}

function outlineExtent(style: CSSStyleDeclaration): number {
  if (style.outlineStyle === 'none')
    return 0
  return parseFloat(style.outlineWidth || '0') + parseFloat(style.outlineOffset || '0')
}

/** How far an element's shadows and outline reach past each of its edges. */
function paintedExtent(style: CSSStyleDeclaration): Extent {
  const shadow = shadowExtent(style.boxShadow)
  const outline = outlineExtent(style)
  return {
    left: Math.max(shadow.left, outline),
    right: Math.max(shadow.right, outline),
    top: Math.max(shadow.top, outline),
    bottom: Math.max(shadow.bottom, outline),
  }
}

/**
 * The edges of `region` that the content around `el` has reached: those that
 * neither the region nor any scroller between it and `el` has scrolled past.
 */
function reachedEdges(el: Element, region: Element): Record<Side, boolean> {
  const reached = { left: true, right: true, top: true, bottom: true }
  for (let node = el.parentElement; node; node = node.parentElement) {
    if (!clips(node))
      continue
    reached.left &&= node.scrollLeft <= 0.5
    reached.right &&= node.scrollLeft + node.clientWidth >= node.scrollWidth - 0.5
    reached.top &&= node.scrollTop <= 0.5
    reached.bottom &&= node.scrollTop + node.clientHeight >= node.scrollHeight - 0.5
    if (node === region)
      break
  }
  return reached
}

/** Whether `el` clips what overflows it, as a scroller or an overflow-hidden box does. */
function clips(el: Element): boolean {
  const s = getComputedStyle(el)
  return s.overflowX !== 'visible' || s.overflowY !== 'visible'
}

export function auditClipping(root: ParentNode = document.body): ClipFinding[] {
  const findings: ClipFinding[] = []
  const regions = [...root.querySelectorAll('*')].filter((el) => clips(el) && el.clientWidth > 0 && el.clientHeight > 0)
  for (const region of regions) {
    const rs = getComputedStyle(region)
    const r = region.getBoundingClientRect()
    const box = {
      left: r.left + parseFloat(rs.borderLeftWidth),
      right: r.right - parseFloat(rs.borderRightWidth),
      top: r.top + parseFloat(rs.borderTopWidth),
      bottom: r.bottom - parseFloat(rs.borderBottomWidth),
    }
    for (const el of region.querySelectorAll('*')) {
      const s = getComputedStyle(el)
      const extent = paintedExtent(s)
      if (Math.max(extent.left, extent.right, extent.top, extent.bottom) <= 0 || s.visibility === 'hidden')
        continue
      const e = el.getBoundingClientRect()
      if (e.width === 0 || e.height === 0)
        continue
      // Only what is currently in view of the region can be judged.
      if (e.bottom < box.top || e.top > box.bottom)
        continue
      // A box that itself leaves the region is a layout matter, not an outline one:
      // only flag an edge whose box is inside while its outline is not.
      // An edge the region, or a scroller between it and the element, has scrolled
      // past clips everything by design; judge only the edges the content has
      // actually reached.
      const reached = reachedEdges(el, region)
      const sides: [Side, boolean, number][] = [
        ['left', reached.left && e.left >= box.left - 0.5, box.left - (e.left - extent.left)],
        ['right', reached.right && e.right <= box.right + 0.5, e.right + extent.right - box.right],
        ['top', reached.top && e.top >= box.top - 0.5, box.top - (e.top - extent.top)],
        ['bottom', reached.bottom && e.bottom <= box.bottom + 0.5, e.bottom + extent.bottom - box.bottom],
      ]
      for (const [side, boxInside, overshoot] of sides) {
        // Sub-pixel rendering makes tiny overshoots meaningless; ring-1 at the edge is 1px.
        if (boxInside && overshoot >= 0.75)
          findings.push({ region, element: el, overshoot, side })
      }
    }
  }
  return findings
}

function describe(el: Element): string {
  const cls = [...el.classList].slice(0, 4).join('.')
  return `${el.tagName.toLowerCase()}${cls ? `.${cls}` : ''}`
}

/** Logs the findings as a table; returns how many there were. */
export function reportClipping(root?: ParentNode): number {
  const findings = auditClipping(root)
  if (findings.length) {
    console.warn(
      `[clip-audit] ${findings.length} outlined element(s) clipped by a scroll region`
    )
    console.table(
      findings.map(
        (f) => ({
          side: f.side,
          overshoot: f.overshoot.toFixed(2),
          element: describe(f.element),
          region: describe(f.region)
        })
      )
    )
  }
  return findings.length
}

declare global {
  interface Window {
    demiAuditClipping: (root?: ParentNode) => ClipFinding[]
  }
}

export function installClipAudit(afterNavigation: (run: () => void) => void): void {
  window.demiAuditClipping = auditClipping
  // Dialogs finish appearing well within this; auditing mid-transition reports their scale.
  if (import.meta.env.DEV)
    afterNavigation(() => window.setTimeout(() => reportClipping(), 1500))
}
