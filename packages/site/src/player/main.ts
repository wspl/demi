// The page each of the site's demos shows in a frame: it plays the demo's
// recording (`recording.ts`) for the screen the frame stands for, so the
// frame shows Demi's own app as it looked, with none of the app's script.
// The page tells it which demo to play (`src/demo/message.ts`); each
// time, the recording starts over.
import { isDemoId, type DemoId } from '@/features'
import { aim } from '@/demo/camera'
import { cameraOf, readPlay, tellCamera, tellReady, tellTab } from '@/demo/message'
import type { RecordedChild, RecordedEvent, RecordedNode, RecordedRange, Recording } from './recording'

const SVG = 'http://www.w3.org/2000/svg'
/**
 * How long an ended animation holds its last frame: longer than the 40ms the
 * recordings show between an animation's end and the change that ends it.
 */
const HOLD_MS = 250
const query = new URLSearchParams(location.search)
const form = query.get('form') === 'phone' ? 'phone' : 'desktop'

/** Answers each demo's recording for this screen, fetched once. */
const recordings = new Map<DemoId, Promise<Recording>>()
function recordingOf(demo: DemoId): Promise<Recording> {
  let recording = recordings.get(demo)
  if (!recording) {
    recording = fetch(`${import.meta.env.BASE_URL}demos/${demo}-${form}.json`).then((response) => {
      if (!response.ok)
        throw new Error(`No recording of ${demo} on the ${form}: run \`bun run record\``)
      return response.json() as Promise<Recording>
    })
    recordings.set(demo, recording)
  }
  return recording
}

/** One play of a recording on this page. */
class Play {
  private readonly nodes = new Map<number, Node>()
  private next = 0
  private stopped = false

  /** Settles once the recording's style sheets have loaded, when its first screen shows. */
  readonly ready: Promise<void>
  private began = 0
  /** Whether the play has shown its first screen, which everything before its start reaches at once. */
  private playing = false
  /**
   * Where the play starts: the demo says it is ready as it mounts, and its
   * first screen reaches the recording in the changes kept a moment later,
   * so a play from the ready alone would show the empty body until then.
   */
  private readonly start: number

  constructor(private readonly recording: Recording) {
    const [rootId, rootAttributes] = recording.root
    const root = document.documentElement
    for (const name of root.getAttributeNames())
      root.removeAttribute(name)
    for (const [name, value] of Object.entries(rootAttributes))
      root.setAttribute(name, value)
    this.nodes.set(rootId, root)
    const opening = openingOf(recording)
    this.start = Math.max(recording.startMs, recording.events[0]?.[0] ?? 0, opening?.[0] ?? 0)
    // A new play shows its opening shot from the start, or the whole screen
    // until its scene points the camera: the stage never shows a shot between.
    const shot = opening ? cameraOf(opening[2])?.rect ?? null : null
    aim(shot, true)
    tellCamera(shot, true)
    this.adopt(document.head, recording.head)
    this.adopt(document.body, recording.body)
    // Everything before the demo's first screen shows at once, styled: the
    // page shows nothing until its sheets apply, and the play starts then.
    veil.replaceSync('html { visibility: hidden; }')
    this.ready = loaded().then(() => {
      if (this.stopped)
        return
      this.until(this.start)
      // The scene set its first screen up before its opening shot, and every
      // transition of it had run by then; endless ones, a spinner's, go on.
      for (const animation of document.getAnimations()) {
        if (Number.isFinite(animation.effect?.getComputedTiming().endTime))
          animation.finish()
      }
      veil.replaceSync('')
      if (this.stopped)
        return
      this.playing = true
      this.began = performance.now()
      requestAnimationFrame(this.tick)
    })
  }

  stop(): void {
    this.stopped = true
  }

  private readonly tick = (): void => {
    if (this.stopped)
      return
    this.until(this.start + performance.now() - this.began)
    if (this.next < this.recording.events.length)
      requestAnimationFrame(this.tick)
  }

  /** The recording's time this play has reached. */
  private at = 0
  /** The recording's animations, in the order of its `animate` events; null for one whose element is gone. */
  private readonly animations: (Animation | null)[] = []
  /** The animations that hold their last frame, with when, in the recording, they end. */
  private held: [Animation, number][] = []

  private until(ms: number): void {
    this.at = ms
    const events = this.recording.events
    let last: number | null = null
    // An animation's event can be timed before the change listed ahead of it.
    while (this.next < events.length && (events[this.next][0] <= ms || events[this.next][1] === 'animate')) {
      const event = events[this.next++]
      // A transition starts from the styles the moment before it: changes
      // the page made in two frames get a style pass between them here too.
      // An animation is timed when it was called, before the change that
      // put its element in the page: it plays in that change's pass.
      if (last !== null && event[0] > last && event[1] !== 'animate')
        void document.body.offsetWidth
      last = event[0]
      this.apply(event)
    }
    // The change that ends an animation comes within a few frames of its end;
    // after them, the element shows as its styles have it, as live.
    this.held = this.held.filter(([animation, end]) => {
      if (ms < end + HOLD_MS)
        return true
      animation.cancel()
      return false
    })
  }

  /** Makes `element` the recorded one: its attributes and its children. */
  private adopt(element: Element, recorded: RecordedNode): void {
    const [id, , attributes, children] = recorded as [number, string, Record<string, string>, RecordedNode[]]
    this.nodes.set(id, element)
    for (const name of element.getAttributeNames())
      element.removeAttribute(name)
    for (const [name, value] of Object.entries(attributes))
      element.setAttribute(name, value)
    settle(element, children.map((child) => this.reused(element, child) ?? this.build(child)))
  }

  /**
   * The style sheet link in `parent` that `recorded` names again, as the
   * head of the play before had it: kept, its sheet is not loaded and
   * applied again, which would show the page unstyled meanwhile.
   */
  private reused(parent: Element, recorded: RecordedNode): Node | null {
    if (recorded[1] !== 'link' || recorded[2].rel !== 'stylesheet')
      return null
    const kept = [...parent.children].find((each) => each instanceof HTMLLinkElement && each.getAttribute('href') === recorded[2].href) ?? null
    if (kept)
      this.nodes.set(recorded[0], kept)
    return kept
  }

  private build(recorded: RecordedNode): Node {
    let node: Node
    if (recorded[1] === 0)
      node = document.createTextNode(recorded[2])
    else if (recorded[1] === 1)
      node = document.createComment('')
    else {
      const [, tag, attributes, children] = recorded
      const element = tag.startsWith('svg:') ? document.createElementNS(SVG, tag.slice(4)) : document.createElement(tag)
      for (const [name, value] of Object.entries(attributes)) {
        try {
          element.setAttribute(name, value)
        } catch {
          // A name the page's framework allows but `setAttribute` does not, such as `@click`.
        }
      }
      element.append(...children.map((child) => this.build(child)))
      node = element
    }
    this.nodes.set(recorded[0], node)
    return node
  }

  private child(child: RecordedChild): Node | undefined {
    return typeof child === 'number' ? this.nodes.get(child) : this.build(child)
  }

  private range([startNode, startOffset, endNode, endOffset]: RecordedRange): Range | null {
    const start = this.nodes.get(startNode)
    const end = this.nodes.get(endNode)
    if (!start || !end)
      return null
    const range = document.createRange()
    try {
      range.setStart(start, startOffset)
      range.setEnd(end, endOffset)
    } catch {
      return null
    }
    return range
  }

  private apply(event: RecordedEvent): void {
    switch (event[1]) {
      case 'children': {
        const parent = this.nodes.get(event[2])
        if (parent instanceof Element || parent instanceof DocumentFragment)
          settle(parent, event[3].map((child) => this.child(child)).filter((child): child is Node => child !== undefined))
        break
      }
      case 'attribute': {
        const element = this.nodes.get(event[2])
        if (element instanceof Element) {
          if (event[4] === null)
            element.removeAttribute(event[3])
          else
            element.setAttribute(event[3], event[4])
        }
        break
      }
      case 'text': {
        const text = this.nodes.get(event[2])
        if (text instanceof CharacterData)
          text.data = event[3]
        break
      }
      case 'scroll': {
        const element = this.nodes.get(event[2])
        if (element instanceof Element) {
          element.scrollTop = event[3]
          element.scrollLeft = event[4]
        }
        break
      }
      case 'value': {
        const field = this.nodes.get(event[2])
        if (field instanceof HTMLInputElement || field instanceof HTMLTextAreaElement)
          field.value = event[3]
        break
      }
      case 'highlight': {
        if (event[3] === null)
          CSS.highlights.delete(event[2])
        else
          CSS.highlights.set(event[2], new Highlight(...event[3].map((each) => this.range(each)).filter((range): range is Range => range !== null)))
        break
      }
      case 'selection': {
        const selection = document.getSelection()
        const range = event[2] && this.range(event[2])
        selection?.removeAllRanges()
        if (range)
          selection?.addRange(range)
        break
      }
      case 'animate': {
        const element = this.nodes.get(event[2])
        if (!(element instanceof Element)) {
          this.animations.push(null)
          break
        }
        // The live page ends an animation with its own change, such as a
        // label's leaving copy removed once it has rolled out; this play may
        // reach that change a frame after the animation ends, so the
        // animation holds its last frame until then.
        const timing: KeyframeAnimationOptions = typeof event[4] === 'number' ? { duration: event[4] } : { ...event[4] }
        // A scroll-driven timeline reaches the recording as an empty object,
        // which `animate` refuses; the play runs it on the document's.
        delete timing.timeline
        // It shows its first frame before it starts, too: the live page
        // started it at the next frame, and its element never showed
        // without it, as a play that starts it late would show it.
        timing.fill = 'both'
        const animation = element.animate(event[3], timing)
        // On from where it was, and on by however late this play applies it.
        if (event[5] !== null)
          animation.currentTime = event[5] + Math.max(0, this.at - event[0])
        this.animations.push(animation)
        const end = animation.effect?.getComputedTiming().endTime
        if (typeof end === 'number' && Number.isFinite(end))
          this.held.push([animation, event[0] - (event[5] ?? 0) + end])
        break
      }
      case 'cancel': {
        this.animations[event[2]]?.cancel()
        break
      }
      case 'message': {
        const data = event[2] as { demi?: string; closed?: unknown } | null
        if (data?.demi === 'tab' && typeof data.closed === 'boolean')
          tellTab(data.closed)
        const shot = cameraOf(data)
        if (shot !== undefined) {
          // A shot the play reaches at once, before its first screen shows, cuts.
          const cut = shot.cut || !this.playing
          aim(shot.rect, cut)
          tellCamera(shot.rect, cut)
        }
        break
      }
    }
  }
}

/**
 * Makes `children` the children of `parent`, moving only the nodes that
 * change place: a node taken out and put back starts its CSS animations
 * and transitions over, as a shimmer's sweep, which the live page did not.
 */
function settle(parent: Node, children: readonly Node[]): void {
  const kept = new Set(children)
  for (const child of [...parent.childNodes]) {
    if (!kept.has(child))
      parent.removeChild(child)
  }
  let at = parent.firstChild
  for (const child of children) {
    if (child === at)
      at = at.nextSibling
    else
      parent.insertBefore(child, at)
  }
}

/**
 * When the scene's opening shot is: a scene sets its state up first, out of
 * view, and the play starts on the shot that cuts to it.
 */
function openingOf(recording: Recording): RecordedEvent | undefined {
  return recording.events.find((event) => event[1] === 'message' && cameraOf(event[2])?.cut === true)
}

/**
 * The sheet that hides the page until its first screen is ready: a sheet of
 * its own, since the recording sets the root's own style attribute.
 */
const veil = new CSSStyleSheet()
document.adoptedStyleSheets = [...document.adoptedStyleSheets, veil]

/**
 * Adds the style sheet links the recording's head has and the page's has not,
 * and settles once they have loaded: the play's head keeps them, so its first
 * screen shows styled at once.
 */
function sheetsOf(recording: Recording): Promise<void> {
  const [, , , children] = recording.head as [number, string, Record<string, string>, RecordedNode[]]
  const have = new Set([...document.head.querySelectorAll('link[rel="stylesheet"]')].map((link) => link.getAttribute('href')))
  const added = children
    .filter((child): child is [number, string, Record<string, string>, RecordedNode[]] => Array.isArray(child) && child[1] === 'link' && child[2].rel === 'stylesheet' && !have.has(child[2].href))
    .map(([, , attributes]) => {
      const link = document.createElement('link')
      link.rel = 'stylesheet'
      link.href = attributes.href
      document.head.append(link)
      return link
    })
  return Promise.all(added.map((link) => new Promise<void>((resolve) => {
    link.addEventListener('load', () => resolve(), { once: true })
    link.addEventListener('error', () => resolve(), { once: true })
  }))).then(() => undefined)
}

/** Settles once every style sheet link of the page has loaded or failed. */
function loaded(): Promise<void> {
  const links = [...document.querySelectorAll<HTMLLinkElement>('link[rel="stylesheet"]')].filter((link) => link.sheet === null)
  return Promise.all(links.map((link) => new Promise<void>((resolve) => {
    link.addEventListener('load', () => resolve(), { once: true })
    link.addEventListener('error', () => resolve(), { once: true })
  }))).then(() => undefined)
}

let playing: Play | null = null
let asked = 0
async function play(demo: DemoId): Promise<void> {
  const ask = ++asked
  const recording = await recordingOf(demo)
  // The screen showing stays until the next is ready to show in its place.
  await sheetsOf(recording)
  if (ask !== asked)
    return
  playing?.stop()
  CSS.highlights.clear()
  document.getSelection()?.removeAllRanges()
  playing = new Play(recording)
  await playing.ready
  // The first screen painted: the page shows the frame from then.
  await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)))
}

readPlay((demo) => void play(demo))
const first = query.get('demo')
void play(first !== null && isDemoId(first) ? first : 'hero').then(tellReady)
