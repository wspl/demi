// The part of `demi.focusRings` that runs in the page. `ringProbe` is sent as
// source (see `frames.ts`), so it uses nothing from outside itself.

/** One part of a focus indicator that does not show. */
export interface RingFinding {
  /** `clipped` by an ancestor, `covered` by an element stacked above, or `hidden`: the element itself is under another, so the focus is out of sight, not only its indicator. */
  kind: 'clipped' | 'covered' | 'hidden'
  /** The ancestor that clips, or the element that covers, as `div.rounded-2xl`. */
  by: string
  /** Why it clips (`overflow: hidden`), or what puts it above (`z-index: 10`). */
  reason: string
  /** How many CSS pixels of the indicator do not show, per side; for `covered`, the indicator's width on each side the sample points found covered, 0 on the others. */
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
  /**
   * The focusable elements that are on the page now, remembered with their unfocused box shadows: inside `root`, or
   * inside the open dialog or menu (the last one that floats over the page), else everywhere. Answers how many, and
   * the kind of layer it limited them to.
   */
  candidates(root: string | null): { count: number, layer: 'dialog' | 'menu' | null }
  /** The element `index` of the candidates, focused, when it is not already the one. */
  focusCandidate(index: number): Promise<boolean>
  /** Whether the focused element is the page itself or one already read: the end of a walk with Tab. */
  seen(): boolean
  /** Reads the focused element's indicator; `none` when it has none, `skipped` when it is not shown or not focused by the keyboard. */
  read(): Promise<RingReading | 'none' | 'skipped'>
  /** Reads the indicator `element` shows now, however it came by it (a selection, a state class): `none` when it shows none. */
  readShown(element: Element): Promise<RingReading | 'none'>
  /** The elements of the candidates the key presses did not reach. */
  unreached(): number[]
}

type Box = { left: number, top: number, right: number, bottom: number }

export function ringProbe(): RingProbe {
  const FOCUSABLE = 'a[href], area[href], button, input:not([type="hidden"]), select, textarea, summary, iframe, audio[controls], video[controls], [contenteditable]:not([contenteditable="false"]), [tabindex]'
  let list: Element[] = []
  let scope: Element | null = null
  const baseline = new WeakMap<Element, string>()
  const read = new WeakSet<Element>()

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
      || style.clipPath !== 'none' || (style.mask !== 'none' && style.mask !== '') || /paint|layout|strict|content/.test(style.contain)
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
    // An <svg> is hit anywhere in its box and paints only what it draws, which hits as its own shapes: only those count.
    if (element instanceof SVGSVGElement) return false
    if (element instanceof HTMLImageElement || element instanceof SVGElement || element instanceof HTMLCanvasElement || element instanceof HTMLVideoElement) return true
    const style = getComputedStyle(element)
    const colour = style.backgroundColor
    return (colour !== 'rgba(0, 0, 0, 0)' && colour !== 'transparent') || style.backgroundImage !== 'none'
  }

  /** The part of a side's reach the indicator covers: from `inner` to `outer` pixels outward of the element's border box (negative: inside it). */
  type Band = { inner: number, outer: number }

  const readIndicator = (element: Element) => {
    const style = getComputedStyle(element)
    const none: Band = { inner: -Infinity, outer: -Infinity }
    const bands = { top: { ...none }, right: { ...none }, bottom: { ...none }, left: { ...none } }
    const add = (side: keyof typeof bands, inner: number, outer: number) => {
      bands[side].inner = Math.max(bands[side].inner, inner)
      bands[side].outer = Math.max(bands[side].outer, outer)
    }
    const parts: string[] = []
    if (style.outlineStyle !== 'none' && px(style.outlineWidth) > 0) {
      parts.push(`outline ${style.outlineWidth}, offset ${style.outlineOffset}`)
      for (const side of ['top', 'right', 'bottom', 'left'] as const) add(side, px(style.outlineOffset), px(style.outlineWidth) + px(style.outlineOffset))
    }
    const before = new Set(shadowRings(baseline.get(element) ?? 'none').map((ring) => ring.text))
    for (const ring of shadowRings(style.boxShadow)) {
      if (before.has(ring.text)) continue
      parts.push(`box-shadow ${ring.spread}px`)
      add('top', 0, ring.spread - ring.y)
      add('bottom', 0, ring.spread + ring.y)
      add('left', 0, ring.spread - ring.x)
      add('right', 0, ring.spread + ring.x)
    }
    // What reaches past the border box is what an ancestor can cut.
    const sides = { top: Math.max(0, bands.top.outer), right: Math.max(0, bands.right.outer), bottom: Math.max(0, bands.bottom.outer), left: Math.max(0, bands.left.outer) }
    return { parts, sides, bands }
  }

  const rounded = (value: number): number => Math.round(value * 10) / 10

  const clippers = (element: Element, indicator: Box): { findings: RingFinding[], region: Box } => {
    const findings: RingFinding[] = []
    // What the clips so far leave of the page: the part of the indicator outside it is not drawn.
    const region: Box = { left: 0, top: 0, right: document.documentElement.clientWidth, bottom: document.documentElement.clientHeight }
    const own = element.getBoundingClientRect()
    let remaining = indicator
    /** Records what `clip` cuts off the indicator that earlier clips left; the sides on which the element's own box is already cut are another defect. */
    const cut = (clip: Box, by: string, reason: string, scrollsX: boolean, scrollsY: boolean): void => {
      region.left = Math.max(region.left, clip.left)
      region.right = Math.min(region.right, clip.right)
      region.top = Math.max(region.top, clip.top)
      region.bottom = Math.min(region.bottom, clip.bottom)
      const cuts = {
        top: own.top >= clip.top - 0.5 ? Math.max(0, clip.top - remaining.top) : 0,
        right: own.right <= clip.right + 0.5 ? Math.max(0, remaining.right - clip.right) : 0,
        bottom: own.bottom <= clip.bottom + 0.5 ? Math.max(0, remaining.bottom - clip.bottom) : 0,
        left: own.left >= clip.left - 0.5 ? Math.max(0, clip.left - remaining.left) : 0,
      }
      remaining = {
        left: Math.max(remaining.left, clip.left), top: Math.max(remaining.top, clip.top),
        right: Math.min(remaining.right, clip.right), bottom: Math.min(remaining.bottom, clip.bottom),
      }
      if (Math.max(cuts.top, cuts.right, cuts.bottom, cuts.left) <= 0.5) return
      findings.push({
        kind: 'clipped', by, reason,
        cuts: { top: rounded(cuts.top), right: rounded(cuts.right), bottom: rounded(cuts.bottom), left: rounded(cuts.left) },
        scrolls: (scrollsX && (cuts.left > 0 || cuts.right > 0)) || (scrollsY && (cuts.top > 0 || cuts.bottom > 0)),
      })
    }
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
      const clipPath = style.clipPath !== 'none'
      if (!clipsX && !clipsY && !clipPath) continue
      const rect = ancestor.getBoundingClientRect()
      const inner = ancestor instanceof HTMLElement
        ? { left: rect.left + ancestor.clientLeft, top: rect.top + ancestor.clientTop, right: rect.left + ancestor.clientLeft + ancestor.clientWidth, bottom: rect.top + ancestor.clientTop + ancestor.clientHeight }
        : rect
      const reasons: string[] = []
      if (style.overflowX !== 'visible' && style.overflowX === style.overflowY) reasons.push(`overflow: ${style.overflowX}`)
      else {
        if (style.overflowX !== 'visible') reasons.push(`overflow-x: ${style.overflowX}`)
        if (style.overflowY !== 'visible') reasons.push(`overflow-y: ${style.overflowY}`)
      }
      if (containsPaint) reasons.push(`contain: ${style.contain}`)
      if (clipPath) reasons.push(`clip-path: ${style.clipPath}`)
      const scrollsX = /auto|scroll/.test(style.overflowX) && ancestor.scrollWidth > ancestor.clientWidth
      const scrollsY = /auto|scroll/.test(style.overflowY) && ancestor.scrollHeight > ancestor.clientHeight
      cut({
        left: clipsX || clipPath ? inner.left : -Infinity, right: clipsX || clipPath ? inner.right : Infinity,
        top: clipsY || clipPath ? inner.top : -Infinity, bottom: clipsY || clipPath ? inner.bottom : Infinity,
      }, short(ancestor), reasons.join(', '), scrollsX, scrollsY)
    }
    cut({ left: 0, top: 0, right: document.documentElement.clientWidth, bottom: document.documentElement.clientHeight }, 'the window', 'edge of the viewport', false, false)
    return { findings, region }
  }

  /** The first opaque element painted above `element` at a point, with why it is above. */
  const aboveAt = (element: Element, x: number, y: number): RingFinding | null => {
    for (const other of document.elementsFromPoint(x, y)) {
      if (other === element || element.contains(other) || other.contains(element)) return null
      if (!opaque(other)) continue
      let common: Element | null = element
      while (common && !common.contains(other)) common = parentOf(common)
      if (!common) return null
      const mine = layer(element, common)
      const theirs = layer(other, common)
      const after = Boolean(element.compareDocumentPosition(other) & Node.DOCUMENT_POSITION_FOLLOWING)
      // In-flow outlines paint after in-flow content; a positioned or stacking layer paints after them, in document order within a z-index.
      if (!(theirs > mine || (theirs === mine && theirs >= 0 && after))) continue
      const style = getComputedStyle(other)
      return {
        kind: 'covered', by: short(other),
        reason: `${style.position}${style.zIndex === 'auto' ? '' : `, z-index: ${style.zIndex}`}`,
        cuts: { top: 0, right: 0, bottom: 0, left: 0 }, scrolls: false,
      }
    }
    return null
  }

  /**
   * The elements painted above the indicator, sampled at three places along each side in the middle of the band the
   * indicator covers: `hidden` when every side is covered, the focused element is then out of sight, not only its ring.
   */
  const covers = (element: Element, region: Box, bands: Record<'top' | 'right' | 'bottom' | 'left', Band>): RingFinding[] => {
    const outer = element.getBoundingClientRect()
    const points: { x: number, y: number, side: keyof typeof bands }[] = []
    for (const fraction of [0.15, 0.5, 0.85]) {
      const across = outer.left + outer.width * fraction
      const along = outer.top + outer.height * fraction
      const middle = (side: keyof typeof bands) => (bands[side].inner + bands[side].outer) / 2
      points.push({ x: across, y: outer.top - middle('top'), side: 'top' })
      points.push({ x: across, y: outer.bottom + middle('bottom'), side: 'bottom' })
      points.push({ x: outer.left - middle('left'), y: along, side: 'left' })
      points.push({ x: outer.right + middle('right'), y: along, side: 'right' })
    }
    const found = new Map<string, RingFinding>()
    const thickness = (side: keyof typeof bands) => bands[side].outer - bands[side].inner
    for (const point of points) {
      // Points an ancestor or the window clips are their finding.
      if (point.x < region.left || point.x >= region.right || point.y < region.top || point.y >= region.bottom) continue
      const above = aboveAt(element, point.x, point.y)
      if (!above) continue
      const finding = found.get(above.by) ?? above
      finding.cuts[point.side] = Math.max(finding.cuts[point.side], thickness(point.side))
      found.set(finding.by, finding)
    }
    for (const finding of found.values()) {
      if (finding.cuts.top > 0 && finding.cuts.right > 0 && finding.cuts.bottom > 0 && finding.cuts.left > 0) finding.kind = 'hidden'
    }
    return [...found.values()]
  }

  const readElement = async (element: Element): Promise<RingReading | 'none'> => {
    // The ring may fade in: it is read where it ends.
    await Promise.allSettled(element.getAnimations().filter((animation) => animation.effect?.getComputedTiming().iterations !== Infinity).map((animation) => animation.finished))
    const { parts, sides, bands } = readIndicator(element)
    if (parts.length === 0) return 'none'
    const rect = element.getBoundingClientRect()
    const box = { left: rect.left - sides.left, top: rect.top - sides.top, right: rect.right + sides.right, bottom: rect.bottom + sides.bottom }
    const clipped = clippers(element, box)
    const findings = [...clipped.findings, ...covers(element, clipped.region, bands)]
    return { element: describe(element), indicator: parts.join(' + '), box, findings }
  }

  return {
    candidates(root) {
      let layer: 'dialog' | 'menu' | null = null
      scope = null
      if (root !== null) {
        scope = document.querySelector(root)
      } else {
        const layers = [...document.querySelectorAll('[role=dialog], [role=alertdialog], [role=menu]')].filter((candidate) => {
          for (let node: Element | null = candidate; node; node = parentOf(node)) {
            if (getComputedStyle(node).position === 'fixed') return shown(candidate)
          }
          return false
        })
        scope = layers.at(-1) ?? null
        if (scope) layer = /dialog/.test(scope.getAttribute('role') ?? '') ? 'dialog' : 'menu'
      }
      const area = scope ?? (root === null ? document : null)
      list = area ? [...area.querySelectorAll(FOCUSABLE)].filter((element) => {
        const disabled = (element as HTMLButtonElement).disabled === true || element.closest('fieldset:disabled') !== null
        return !disabled && shown(element) && !element.closest('[inert]')
      }) : []
      for (const element of list) baseline.set(element, getComputedStyle(element).boxShadow)
      return { count: list.length, layer }
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
      if (scope !== null && !scope.contains(element)) return 'skipped'
      if (!element.matches(':focus-visible') || !shown(element)) return 'skipped'
      return readElement(element)
    },
    readShown: (element) => readElement(element),
    unreached() {
      return list.flatMap((element, index) => (read.has(element) ? [] : [index]))
    },
  }
}
