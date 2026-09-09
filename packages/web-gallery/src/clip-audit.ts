/**
 * Finds outlined elements that a scroll region would clip. A region with
 * `overflow-y: auto` clips the x axis too, so a ring, focus ring or corner badge on
 * a child flush with the region's edge is cut off. See docs/demi-next/web-prototype.md.
 */
export interface ClipFinding {
  region: Element
  element: Element
  /** How far, in CSS pixels, the outline reaches past the region's padding box. */
  overshoot: number
  side: 'left' | 'right' | 'top' | 'bottom'
}

/** The largest distance a box-shadow list reaches outside the box. */
function shadowExtent(boxShadow: string): number {
  if (!boxShadow || boxShadow === 'none')
    return 0
  let extent = 0
  // Colors may contain commas inside rgb(); split on commas that follow a length.
  for (const shadow of boxShadow.split(/,(?![^(]*\))/)) {
    if (/\binset\b/.test(shadow))
      continue
    const lengths = shadow.match(/-?\d*\.?\d+px/g)?.map((v) => parseFloat(v)) ?? []
    const [x = 0, y = 0, blur = 0, spread = 0] = lengths
    extent = Math.max(extent, Math.max(Math.abs(x), Math.abs(y)) + blur + spread)
  }
  return extent
}

function outlineExtent(style: CSSStyleDeclaration): number {
  if (style.outlineStyle === 'none')
    return 0
  return parseFloat(style.outlineWidth || '0') + parseFloat(style.outlineOffset || '0')
}

export function auditClipping(root: ParentNode = document.body): ClipFinding[] {
  const findings: ClipFinding[] = []
  const regions = [...root.querySelectorAll('*')].filter((el) => {
    const s = getComputedStyle(el)
    return (s.overflowX !== 'visible' || s.overflowY !== 'visible') &&
      el.clientWidth > 0 &&
      el.clientHeight > 0
  })
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
      const extent = Math.max(shadowExtent(s.boxShadow), outlineExtent(s))
      if (extent <= 0 || s.visibility === 'hidden')
        continue
      const e = el.getBoundingClientRect()
      if (e.width === 0 || e.height === 0)
        continue
      // Only what is currently in view of the region can be judged.
      if (e.bottom < box.top || e.top > box.bottom)
        continue
      // A box that itself leaves the region is a layout matter, not an outline one:
      // only flag an edge whose box is inside while its outline is not.
      // An edge the region has scrolled past clips everything by design; judge only the
      // edges the content has actually reached.
      const atLeft = region.scrollLeft <= 0.5
      const atRight = region.scrollLeft + region.clientWidth >= region.scrollWidth - 0.5
      const atTop = region.scrollTop <= 0.5
      const atBottom = region.scrollTop + region.clientHeight >= region.scrollHeight - 0.5
      const sides: [ClipFinding['side'], boolean, number][] = [
        [
          'left',
          atLeft && e.left >= box.left - 0.5,
          box.left - (e.left - extent)
        ],
        [
          'right',
          atRight && e.right <= box.right + 0.5,
          e.right + extent - box.right
        ],
        ['top', atTop && e.top >= box.top - 0.5, box.top - (e.top - extent)],
        [
          'bottom',
          atBottom && e.bottom <= box.bottom + 0.5,
          e.bottom + extent - box.bottom
        ],
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
