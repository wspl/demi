// What `record.ts` runs in the demo's page before the page's own script:
// it keeps the page's first snapshot and each change after it in
// `window.recording`, as `src/player/recording.ts` describes.
import type { RecordedEvent, RecordedNode, RecordedRange, Recording } from '../src/player/recording'
import './recorder-types'

const SVG = 'http://www.w3.org/2000/svg'
function start(): void {
  const ids = new WeakMap<Node, number>()
  let nextId = 1
  const idOf = (node: Node): number => {
    let id = ids.get(node)
    if (id === undefined) {
      id = nextId++
      ids.set(node, id)
    }
    return id
  }
  const known = new WeakSet<Node>()
  // The recording plays on another server: its own addresses become paths.
  const local = (value: string): string => value.replaceAll(location.origin, '')

  // Scripts and preloads stay out: the recording plays with no script of the demo's.
  const kept = (node: Node): boolean => {
    if (!(node instanceof Element))
      return node.nodeType === Node.TEXT_NODE || node.nodeType === Node.COMMENT_NODE
    const tag = node.localName
    return tag !== 'script' && !(tag === 'link' && /preload/.test(node.getAttribute('rel') ?? ''))
  }
  const snapshot = (node: Node): RecordedNode => {
    known.add(node)
    if (node.nodeType === Node.TEXT_NODE)
      return [idOf(node), 0, node.parentNode instanceof HTMLStyleElement ? local((node as Text).data) : (node as Text).data]
    if (!(node instanceof Element))
      return [idOf(node), 1]
    const attributes: Record<string, string> = {}
    for (const attribute of node.attributes)
      attributes[attribute.name] = local(attribute.value)
    const children = [...node.childNodes].filter(kept).map(snapshot)
    return [idOf(node), node.namespaceURI === SVG ? `svg:${node.localName}` : node.localName, attributes, children]
  }
  const rangeOf = (range: AbstractRange): RecordedRange =>
    [idOf(range.startContainer), range.startOffset, idOf(range.endContainer), range.endOffset]

  const root = document.documentElement
  const rootAttributes: Record<string, string> = {}
  for (const attribute of root.attributes)
    rootAttributes[attribute.name] = attribute.value
  const began = performance.now()
  const events: RecordedEvent[] = []
  const now = (): number => Math.round(performance.now() - began)
  // Every change one task of the page makes is timed when the task made its
  // first: the page paints none of them before the task ends, so a play
  // applies them in one pass. Timed apart, a render and the callback after
  // it that starts the rendered row's roll would be painted apart, the row
  // showing its new face before the roll.
  let taskMs: number | null = null
  const taskEnd = new MessageChannel()
  taskEnd.port1.onmessage = () => {
    taskMs = null
  }
  const stamp = (): number => {
    if (taskMs === null) {
      taskMs = now()
      taskEnd.port2.postMessage(null)
    }
    return taskMs
  }
  const recording: Recording = {
    root: [idOf(root), rootAttributes],
    head: snapshot(document.head),
    body: snapshot(document.body),
    startMs: 0,
    events,
  }
  known.add(root)
  window.recording = recording

  const keepMutations = (mutations: MutationRecord[]): void => {
    if (mutations.length === 0)
      return
    const ms = stamp()
    const parents = new Set<Node>()
    for (const mutation of mutations) {
      if (mutation.type === 'childList')
        parents.add(mutation.target)
    }
    for (const parent of parents) {
      if (!parent.isConnected || !known.has(parent))
        continue
      const children = [...parent.childNodes].filter(kept).map((child) => (known.has(child) ? idOf(child) : snapshot(child)))
      events.push([ms, 'children', idOf(parent), children])
    }
    for (const mutation of mutations) {
      const target = mutation.target
      if (!target.isConnected || !known.has(target))
        continue
      if (mutation.type === 'attributes' && target instanceof Element) {
        const value = target.getAttribute(mutation.attributeName!)
        events.push([ms, 'attribute', idOf(target), mutation.attributeName!, value === null ? null : local(value)])
      } else if (mutation.type === 'characterData')
        events.push([ms, 'text', idOf(target), (target as Text).data])
    }
  }
  // The animations of the elements a change put in the page are kept right
  // after it, so a play starts them in the pass that shows the elements.
  const observer = new MutationObserver((mutations) => {
    keepMutations(mutations)
    keepStarted()
  })
  observer.observe(root, { childList: true, subtree: true, attributes: true, characterData: true })
  /**
   * Keeps the changes made before now that the observer has not reported yet:
   * a scroll or a cancel kept now comes after them, as it did in the page.
   */
  const keepPending = (): void => keepMutations(observer.takeRecords())

  const keepScroll = (element: Element): void => {
    keepPending()
    if (known.has(element))
      events.push([stamp(), 'scroll', idOf(element), Math.round(element.scrollTop), Math.round(element.scrollLeft)])
  }
  document.addEventListener('scroll', (event) => {
    keepScroll(event.target === document ? document.scrollingElement! : (event.target as Element))
  }, true)
  // A script's scroll is kept when the script makes it, with the changes and
  // the animations it goes with, such as rows gliding from where the scroll
  // moved them: its scroll event comes a frame later, and a play that
  // scrolled then would show those rows offset for that frame.
  for (const name of ['scrollTop', 'scrollLeft'] as const) {
    const property = Object.getOwnPropertyDescriptor(Element.prototype, name)!
    Object.defineProperty(Element.prototype, name, {
      ...property,
      set(this: Element, value: number) {
        property.set!.call(this, value)
        keepScroll(this)
      },
    })
  }
  for (const name of ['scrollTo', 'scrollBy'] as const) {
    const scroll = Element.prototype[name]
    Element.prototype[name] = function (this: Element, ...args: Parameters<typeof scroll>) {
      scroll.apply(this, args)
      keepScroll(this)
    } as typeof scroll
  }
  document.addEventListener('input', (event) => {
    const field = event.target
    if ((field instanceof HTMLInputElement || field instanceof HTMLTextAreaElement) && known.has(field))
      events.push([stamp(), 'value', idOf(field), field.value])
  }, true)
  document.addEventListener('selectionchange', () => {
    const selection = document.getSelection()
    const range = selection && selection.rangeCount > 0 && !selection.isCollapsed ? selection.getRangeAt(0) : null
    events.push([stamp(), 'selection', range ? rangeOf(range) : null])
  })

  // Animations the script runs (a label rolling over, a spinner's tail) are
  // no change to the page: each is kept as the call that started it, once
  // its element is in the recording, which may be after the call returns.
  const animate = Element.prototype.animate
  // Each is timed when it was called, as the change that put its element in
  // the page was, so the two play in the same frame: the element never shows
  // without its animation.
  const started: [task: number, called: number, element: Element, keyframes: Keyframe[] | PropertyIndexedKeyframes | null, options: number | KeyframeAnimationOptions | undefined, animation: Animation][] = []
  /** The kept animations, by their places among the recording's `animate` events. */
  const animations = new Map<Animation, number>()
  /** The animations cancelled before they were kept, which the recording leaves out. */
  const cancelled = new WeakSet<Animation>()
  const keepStarted = (): void => {
    for (const [task, called, element, keyframes, options, animation] of started.splice(0)) {
      if (!known.has(element) || !keyframes || animation.playState === 'finished' || cancelled.has(animation))
        continue
      animations.set(animation, animations.size)
      const current = animation.currentTime
      events.push([
        task,
        'animate',
        idOf(element),
        JSON.parse(JSON.stringify(keyframes)),
        options === undefined ? undefined : JSON.parse(JSON.stringify(options)),
        typeof current === 'number' ? Math.round(current - (now() - called)) : null,
      ])
    }
  }
  // A cancel is kept when it is made: a glide that replaces a running one
  // cancels it, and a play that ran both would offset its rows twice.
  const cancel = Animation.prototype.cancel
  Animation.prototype.cancel = function (this: Animation) {
    cancel.call(this)
    keepPending()
    const index = animations.get(this)
    if (index === undefined)
      cancelled.add(this)
    else
      events.push([stamp(), 'cancel', index])
  }
  Element.prototype.animate = function (keyframes, options) {
    if (started.length === 0)
      setTimeout(keepStarted)
    const animation = animate.call(this, keyframes, options)
    started.push([stamp(), now(), this, keyframes, options, animation])
    return animation
  }

  const highlights = CSS.highlights
  const set = highlights.set.bind(highlights)
  const remove = highlights.delete.bind(highlights)
  highlights.set = (name, highlight) => {
    events.push([stamp(), 'highlight', name, [...highlight].map(rangeOf)])
    return set(name, highlight)
  }
  highlights.delete = (name) => {
    events.push([stamp(), 'highlight', name, null])
    return remove(name)
  }

  const post = window.parent.postMessage.bind(window.parent) as (message: unknown, origin: string) => void
  window.parent.postMessage = ((message: unknown, origin: string) => {
    const data = message as { demi?: string } | null
    // The demo's first screen is up when it says it is ready: the recording plays from there.
    if (data?.demi === 'ready')
      recording.startMs = now()
    else
      events.push([stamp(), 'message', message])
    post(message, origin)
  }) as typeof window.postMessage
}

// The recording starts with the body, before the demo's script renders into it.
if (document.body)
  start()
else {
  new MutationObserver((_, observer) => {
    if (!document.body)
      return
    observer.disconnect()
    start()
  }).observe(document, { childList: true, subtree: true })
}
