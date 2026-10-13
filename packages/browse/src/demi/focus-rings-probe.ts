// The part of `demi.focusRings` that runs in the page. `ringProbe` is sent as
// source (see `frames.ts`), so it uses nothing from outside itself.

/** One part of a focus indicator that does not show. */
export interface RingFinding {
  /** `clipped` by an ancestor, or `covered` by an element stacked above. */
  kind: 'clipped' | 'covered'
  /** The ancestor that clips, or the element that covers, as `div.rounded-2xl`. */
  by: string
  /** Why it clips (`overflow: hidden`), or what puts it above (`z-index: 10`). */
  reason: string
  /** How many CSS pixels of the indicator do not show, per side; for `covered`, 0 for the sides the sample points found free. */
  cuts: { top: number, right: number, bottom: number, left: number }
  /** The clipping ancestor scrolls on an axis it clips, so the cut depends on where it is scrolled. */
  scrolls: boolean
}

export interface RingReading {
  /** The element, as `button "image.png" .attachment-capsule`. */
  element: string
  /** The indicator, such as `outline 2px, offset 1px`. */
  indicator: string
  /** Where the indicator ends, in CSS pixels of the viewport. */
  box: { left: number, top: number, right: number, bottom: number }
  findings: RingFinding[]
}

export interface RingProbe {
  /** The focusable elements that are on the page now, remembered with their unfocused box shadows. */
  candidates(root: string | null): number
  /** The element `index` of the candidates, focused, when it is not already the one. */
  focusCandidate(index: number): Promise<boolean>
  /** Whether the focused element is the page itself or one already read: the end of a walk with Tab. */
  seen(): boolean
  /** Reads the focused element's indicator; null when it has none, `skipped` when it is not shown or not focused by the keyboard. */
  read(): Promise<RingReading | 'none' | 'skipped'>
  /** The elements of the candidates the key presses did not reach. */
  unreached(): number[]
  /** Forgets the elements read. */
  reset(): void
}

export function ringProbe(): RingProbe {
  const FOCUSABLE = 'a[href], area[href], button, input:not([type="hidden"]), select, textarea, summary, iframe, audio[controls], video[controls], [contenteditable]:not([contenteditable="false"]), [tabindex]'
  let list: Element[] = []
  let scope: string | null = null
  const baseline = new WeakMap<Element, string>()
  let read = new WeakSet<Element>()

  const describe = (element: Element): string => {
    const tag = element.tagName.toLowerCase()
    const label = (element.getAttribute('aria-label') ?? element.getAttribute('title') ?? element.getAttribute('placeholder') ?? element.textContent ?? '').replace(/\s+/g, ' ').trim()
    const role = element.getAttribute('role')
    const classes = [...element.classList].slice(0, 3).map((name) => `.${name}`).join('')
    const id = element.id ? `#${element.id}` : ''
    return `${role ?? tag}${label ? ` "${label.length > 40 ? `${label.slice(0, 39)}…` : label}"` : ''} ${tag}${id}${classes}`.trim()
  }
  const short = (element: Element): string => {
    const classes = [...element.classList].slice(0, 3).map((name) => `.${name}`).join('')
    return `${element.tagName.toLowerCase()}${element.id ? `#${element.id}` : ''}${classes}`
  }
  const parentOf = (element: Element): Element | null =>
    element.parentElement ?? (element.getRootNode() instanceof ShadowRoot ? (element.getRootNode() as ShadowRoot).host : null)
  const px = (value: string): number => Number.parseFloat(value) || 0
  const shown = (element: Element): boolean => {
    const box = element.getBoundingClientRect()
    const style = getComputedStyle(element)
    return box.width > 0 && box.height > 0 && style.visibility === 'visible' && style.display !== 'none'
  }

  /** The non-inset, crisp layers of a `box-shadow` value, as spread rings. */
  const shadowRings = (value: string): { x: number, y: number, spread: number, text: string }[] => {
    const layers: string[] = []
    let depth = 0
    let start = 0
    for (let index = 0; index < value.length; index++) {
      const character = value[index]
      if (character === '(') depth++
      else if (character === ')') depth--
      else if (character === ',' && depth === 0) {
        layers.push(value.slice(start, index).trim())
        start = index + 1
      }
    }
    layers.push(value.slice(start).trim())
    const rings: { x: number, y: number, spread: number, text: string }[] = []
    for (const layer of layers) {
      if (layer === 'none' || layer.includes('inset')) continue
      const numbers = [...layer.replace(/(rgba?|color|oklch|oklab|hsla?|lab|lch)\([^)]*\)/g, '').matchAll(/-?\d*\.?\d+px|(?<![\w.])0(?![\w.])/g)].map((match) => px(match[0]))
      const [x = 0, y = 0, blur = 0, spread = 0] = numbers
      if (blur === 0 && spread > 0) rings.push({ x, y, spread, text: layer })
    }
    return rings
  }

  const isStackingContext = (style: CSSStyleDeclaration, parentStyle: CSSStyleDeclaration | null): boolean => {
    const positioned = style.position !== 'static'
    return (style.zIndex !== 'auto' && (positioned || (parentStyle !== null && /flex|grid/.test(parentStyle.display))))
      || style.position === 'fixed' || style.position === 'sticky'
      || Number(style.opacity) < 1 || style.transform !== 'none' || style.filter !== 'none'
      || style.backdropFilter !== 'none' || style.isolation === 'isolate' || style.mixBlendMode !== 'normal'
      || style.clipPath !== 'none' || style.mask !== 'none' && style.mask !== '' || /paint|layout|strict|content/.test(style.contain)
  }

  /**
   * Where the chain from `node` up to (not including) `top` paints: -1 for
   * in-flow content, whose outlines the browser paints after it, else the
   * z-index of the layer of the chain's outermost positioned or stacking
   * element (auto as 0).
   */
  const layer = (node: Element, top: Element): number => {
    let found: number | null = null
    for (let current: Element | null = node; current && current !== top; current = parentOf(current)) {
      const style = getComputedStyle(current)
      const parent = parentOf(current)
      if (style.position !== 'static' || isStackingContext(style, parent ? getComputedStyle(parent) : null)) {
        found = style.zIndex === 'auto' ? 0 : Number(style.zIndex)
      }
    }
    return found ?? -1
  }

  const opaque = (element: Element): boolean => {
    if (element instanceof HTMLImageElement || element instanceof SVGElement || element instanceof HTMLCanvasElement || element instanceof HTMLVideoElement) return true
    const style = getComputedStyle(element)
    const colour = style.backgroundColor
    return (colour !== 'rgba(0, 0, 0, 0)' && colour !== 'transparent') || style.backgroundImage !== 'none'
  }

  const readIndicator = (element: Element) => {
    const style = getComputedStyle(element)
    const sides = { top: 0, right: 0, bottom: 0, left: 0 }
    const parts: string[] = []
    if (style.outlineStyle !== 'none' && px(style.outlineWidth) > 0) {
      const reach = px(style.outlineWidth) + px(style.outlineOffset)
      parts.push(`outline ${style.outlineWidth}, offset ${style.outlineOffset}`)
      for (const side of ['top', 'right', 'bottom', 'left'] as const) sides[side] = Math.max(sides[side], reach)
    }
    const before = new Set(shadowRings(baseline.get(element) ?? 'none').map((ring) => ring.text))
    for (const ring of shadowRings(style.boxShadow)) {
      if (before.has(ring.text)) continue
      parts.push(`box-shadow ${ring.spread}px`)
      sides.top = Math.max(sides.top, ring.spread - ring.y)
      sides.bottom = Math.max(sides.bottom, ring.spread + ring.y)
      sides.left = Math.max(sides.left, ring.spread - ring.x)
      sides.right = Math.max(sides.right, ring.spread + ring.x)
    }
    return { parts, sides }
  }

  const clippers = (element: Element, box: { left: number, top: number, right: number, bottom: number }): { findings: RingFinding[], region: { left: number, top: number, right: number, bottom: number } } => {
    const findings: RingFinding[] = []
    // What every clip leaves of the page: the points of the ring outside it are not drawn.
    const region = { left: 0, top: 0, right: document.documentElement.clientWidth, bottom: document.documentElement.clientHeight }
    const subject = getComputedStyle(element)
    // An absolutely placed element is clipped only from its containing block outward, a fixed one by none.
    let waiting = subject.position === 'absolute' || subject.position === 'fixed'
    let fixed = subject.position === 'fixed'
    for (let ancestor = parentOf(element); ancestor; ancestor = parentOf(ancestor)) {
      const style = getComputedStyle(ancestor)
      if (waiting) {
        const contains = style.position !== 'static' || style.transform !== 'none' || style.filter !== 'none' || /paint|layout|strict|content/.test(style.contain)
        if (!contains && !(fixed && ancestor === document.documentElement)) continue
        waiting = false
        fixed = false
      }
      if (style.position === 'absolute' || style.position === 'fixed') {
        waiting = true
        fixed = style.position === 'fixed'
      }
      const containsPaint = /paint|strict|content/.test(style.contain)
      const clipsX = style.overflowX !== 'visible' || containsPaint
      const clipsY = style.overflowY !== 'visible' || containsPaint
      if (!clipsX && !clipsY && style.clipPath === 'none') continue
      const rect = ancestor.getBoundingClientRect()
      const inner = ancestor instanceof HTMLElement || ancestor instanceof SVGElement
        ? {
          left: rect.left + (ancestor as HTMLElement).clientLeft, top: rect.top + (ancestor as HTMLElement).clientTop,
          right: rect.left + (ancestor as HTMLElement).clientLeft + (ancestor as HTMLElement).clientWidth,
          bottom: rect.top + (ancestor as HTMLElement).clientTop + (ancestor as HTMLElement).clientHeight,
        }
        : rect
      const clip = {
        left: clipsX || style.clipPath !== 'none' ? inner.left : -Infinity,
        right: clipsX || style.clipPath !== 'none' ? inner.right : Infinity,
        top: clipsY || style.clipPath !== 'none' ? inner.top : -Infinity,
        bottom: clipsY || style.clipPath !== 'none' ? inner.bottom : Infinity,
      }
      region.left = Math.max(region.left, clip.left)
      region.right = Math.min(region.right, clip.right)
      region.top = Math.max(region.top, clip.top)
      region.bottom = Math.min(region.bottom, clip.bottom)
      const cuts = {
        top: Math.max(0, clip.top - box.top),
        right: Math.max(0, box.right - clip.right),
        bottom: Math.max(0, box.bottom - clip.bottom),
        left: Math.max(0, clip.left - box.left),
      }
      if (Math.max(cuts.top, cuts.right, cuts.bottom, cuts.left) < 0.5) continue
      const reasons: string[] = []
      if (style.overflowX !== 'visible') reasons.push(`overflow-x: ${style.overflowX}`)
      if (style.overflowY !== 'visible') reasons.push(`overflow-y: ${style.overflowY}`)
      if (containsPaint) reasons.push(`contain: ${style.contain}`)
      if (style.clipPath !== 'none') reasons.push(`clip-path: ${style.clipPath}`)
      const scrolls = ((style.overflowX === 'auto' || style.overflowX === 'scroll') && ancestor.scrollWidth > ancestor.clientWidth)
        || ((style.overflowY === 'auto' || style.overflowY === 'scroll') && ancestor.scrollHeight > ancestor.clientHeight)
      findings.push({
        kind: 'clipped',
        by: short(ancestor),
        reason: reasons.join(', ').replace(/(overflow-x: (\w+), overflow-y: \2)/, 'overflow: $2'),
        cuts: Object.fromEntries(Object.entries(cuts).map(([side, amount]) => [side, Math.round(amount * 10) / 10])) as RingFinding['cuts'],
        scrolls,
      })
    }
    // The window's edge cuts too.
    const viewport = { top: 0, left: 0, right: document.documentElement.clientWidth, bottom: document.documentElement.clientHeight }
    const cuts = {
      top: Math.max(0, viewport.top - box.top), right: Math.max(0, box.right - viewport.right),
      bottom: Math.max(0, box.bottom - viewport.bottom), left: Math.max(0, viewport.left - box.left),
    }
    if (Math.max(cuts.top, cuts.right, cuts.bottom, cuts.left) >= 0.5) {
      findings.push({ kind: 'clipped', by: 'the window', reason: 'edge of the viewport', cuts: Object.fromEntries(Object.entries(cuts).map(([side, amount]) => [side, Math.round(amount * 10) / 10])) as RingFinding['cuts'], scrolls: false })
    }
    return { findings, region }
  }

  const covers = (element: Element, region: { left: number, top: number, right: number, bottom: number }, ring: { top: number, right: number, bottom: number, left: number }): RingFinding[] => {
    const outer = element.getBoundingClientRect()
    // Sample points in the middle of the ring band along each side: at a quarter, the middle and three quarters.
    const points: { x: number, y: number, side: keyof typeof ring }[] = []
    for (const fraction of [0.15, 0.5, 0.85]) {
      const across = outer.left + outer.width * fraction
      const along = outer.top + outer.height * fraction
      if (ring.top > 0) points.push({ x: across, y: outer.top - ring.top / 2, side: 'top' })
      if (ring.bottom > 0) points.push({ x: across, y: outer.bottom + ring.bottom / 2, side: 'bottom' })
      if (ring.left > 0) points.push({ x: outer.left - ring.left / 2, y: along, side: 'left' })
      if (ring.right > 0) points.push({ x: outer.right + ring.right / 2, y: along, side: 'right' })
    }
    const found = new Map<Element, RingFinding>()
    for (const point of points) {
      // Points an ancestor or the window clips are their finding.
      if (point.x < region.left || point.x >= region.right || point.y < region.top || point.y >= region.bottom) continue
      for (const other of document.elementsFromPoint(point.x, point.y)) {
        if (other === element || element.contains(other) || other.contains(element)) break
        if (!opaque(other)) continue
        let common: Element | null = element
        while (common && !common.contains(other)) common = parentOf(common)
        if (!common) break
        const mine = layer(element, common)
        const theirs = layer(other, common)
        const after = Boolean(element.compareDocumentPosition(other) & Node.DOCUMENT_POSITION_FOLLOWING)
        // In-flow outlines paint after in-flow content; a positioned or stacking layer paints after them, in document order within a z-index.
        const above = theirs > mine || (theirs === mine && theirs >= 0 && after)
        if (!above) continue
        const style = getComputedStyle(other)
        const finding = found.get(other) ?? {
          kind: 'covered' as const,
          by: short(other),
          reason: `${style.position}${style.zIndex === 'auto' ? '' : `, z-index: ${style.zIndex}`}`,
          cuts: { top: 0, right: 0, bottom: 0, left: 0 },
          scrolls: false,
        }
        finding.cuts[point.side] = Math.max(finding.cuts[point.side], ring[point.side])
        found.set(other, finding)
        break
      }
    }
    return [...found.values()]
  }

  return {
    candidates(root) {
      scope = root
      const area = root === null ? document : document.querySelector(root)
      list = area ? [...area.querySelectorAll(FOCUSABLE)].filter((element) => {
        const disabled = (element as HTMLButtonElement).disabled === true || element.closest('fieldset:disabled') !== null
        return !disabled && shown(element) && !element.closest('[inert]')
      }) : []
      for (const element of list) baseline.set(element, getComputedStyle(element).boxShadow)
      return list.length
    },
    async focusCandidate(index) {
      const element = list[index]
      if (!element || read.has(element)) return false
      element.scrollIntoView({ block: 'nearest', inline: 'nearest' })
      ;(element as HTMLElement).focus()
      return document.activeElement === element
    },
    seen() {
      const active = document.activeElement
      return !active || active === document.body || read.has(active)
    },
    async read() {
      const element = document.activeElement
      if (!element || element === document.body) return 'skipped'
      read.add(element)
      if (scope !== null && !element.closest(scope)) return 'skipped'
      if (!element.matches(':focus-visible') || !shown(element)) return 'skipped'
      // The ring may fade in: it is read where it ends.
      await Promise.allSettled(element.getAnimations().filter((animation) => animation.effect?.getComputedTiming().iterations !== Infinity).map((animation) => animation.finished))
      const { parts, sides } = readIndicator(element)
      if (parts.length === 0) return 'none'
      const rect = element.getBoundingClientRect()
      const box = { left: rect.left - sides.left, top: rect.top - sides.top, right: rect.right + sides.right, bottom: rect.bottom + sides.bottom }
      const clipped = clippers(element, box)
      const findings = [...clipped.findings, ...covers(element, clipped.region, sides)]
      return { element: describe(element), indicator: parts.join(' + '), box, findings }
    },
    unreached() {
      return list.flatMap((element, index) => (read.has(element) ? [] : [index]))
    },
    reset() {
      list = []
      read = new WeakSet<Element>()
    },
  }
}
