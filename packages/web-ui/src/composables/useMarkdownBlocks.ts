import { computed, effectScope, onScopeDispose, shallowRef, watch, type ComputedRef, type EffectScope } from 'vue'
import { messageBlockEnds } from '../markdown/render'
import { closeOpenInlineMarkdown, wrapSteps, type RevealStep } from '../ui/stream-reveal'

/** A block of a reply as the page shows it. */
interface Block<T> {
  /** Where its text starts in the reply's. */
  start: number
  /** Its text: fixed for a settled block, the rest of the reply for the last one. */
  text: () => string
  /** Its render, set up in `scope`, which ends with the block. */
  html: ComputedRef<string>
  scope: EffectScope
  /** What shows of it: the render its nodes are of, and what `describe` found in them. */
  shown: { html: string; nodes: ChildNode[]; found: T } | null
}

export interface MarkdownBlocksOptions<T> {
  /** The element the reply renders into, whose children this keeps. */
  root: () => HTMLElement | undefined
  /** The reply's text that shows. */
  text: () => string
  streaming: () => boolean
  /** The reply's steps still fading in (`useStreamReveal`). */
  steps: () => readonly RevealStep[]
  /**
   * The HTML of the Markdown `source` gives, sanitized: set up in a block's
   * own scope, which ends with the block, and read again when what it reads
   * changes.
   */
  render: (source: () => string) => () => string
  /** What the page needs of a block's nodes, found each time it renders; `fresh` are those the render added. */
  describe: (nodes: readonly ChildNode[], fresh: readonly ChildNode[]) => T
  /** Called after each change of the page with what `describe` found in each block, in order. */
  rendered: (found: readonly T[]) => void
}

/**
 * The text of each node a render added, as the render made it, before a
 * fade wrapped any of its text: a node the next render makes the same stays
 * in the page, with its fades going on, and only the others are replaced.
 */
const renderedAs = new WeakMap<ChildNode, string>()

/**
 * Renders a reply into `root` a block at a time (`file-previews.md`, a reply
 * that streams renders a block at a time): while it streams, every block
 * `messageBlockEnds` settles renders once and stays as the same nodes, and
 * only the last block renders again as text arrives; otherwise the reply
 * renders whole. Each render of a block replaces only its nodes that
 * differ, so when the stream ends and the reply renders whole, the blocks
 * that render as they did stay as they are. A fresh render fades in the
 * steps still fading within it, before it reaches the page.
 */
export function useMarkdownBlocks<T>(options: MarkdownBlocksOptions<T>): void {
  const blocks = shallowRef<Block<T>[]>([])
  /** The settled blocks' text, which the reply's text starts with while it streams; null while it renders whole. */
  let settledText: string | null = null

  function newBlock(start: number, text: () => string, last: boolean): Block<T> {
    const scope = effectScope(true)
    // Closers apply to finished text too: an aborted stream leaves the same half-open markers.
    const source = last ? () => text() + closeOpenInlineMarkdown(text()) : text
    const html = scope.run(() => computed(options.render(source)))!
    return { start, text, html, scope, shown: null }
  }

  function lastBlock(start: number): Block<T> {
    return newBlock(start, () => options.text().slice(start), true)
  }

  function replace(next: Block<T>[]): void {
    for (const old of blocks.value) {
      if (!next.includes(old))
        old.scope.stop()
    }
    blocks.value = next
  }

  function update(): void {
    const text = options.text()
    if (!options.streaming()) {
      if (settledText !== null || blocks.value.length === 0)
        replace([lastBlock(0)])
      settledText = null
      return
    }
    // A text that no longer starts with the settled blocks, rewritten, starts over.
    const continues = settledText !== null && text.startsWith(settledText)
    const settled = continues ? blocks.value.slice(0, -1) : []
    const from = continues ? settledText!.length : 0
    const ends = messageBlockEnds(text.slice(from))
    // Text that settles nothing is the last block's, which reads it itself.
    if (continues && ends.length === 0)
      return
    let start = from
    for (const end of ends) {
      const source = text.slice(start, from + end)
      settled.push(newBlock(start, () => source, false))
      start = from + end
    }
    settledText = text.slice(0, start)
    replace([...settled, lastBlock(start)])
  }

  /** Whether `block` shows its render now, its nodes starting at `at` when given. */
  function showsRender(block: Block<T>, at?: ChildNode | null): boolean {
    const shown = block.shown
    if (!shown || shown.html !== block.html.value)
      return false
    const first = shown.nodes[0]
    return first === undefined || (at === undefined ? first.parentNode === options.root() : first === at)
  }

  /** The nodes of a fresh render of `block`, with the steps fading within it wrapped. */
  function render(block: Block<T>, steps: readonly RevealStep[]): ChildNode[] {
    const holder = document.createElement('div')
    holder.innerHTML = block.html.value
    for (const node of holder.childNodes)
      renderedAs.set(node, node instanceof Element ? node.outerHTML : `${node.nodeType}:${node.textContent}`)
    const within = steps
      .filter((step) => step.end > block.start)
      .map((step) => ({ ...step, start: Math.max(0, step.start - block.start), end: step.end - block.start }))
    wrapSteps(holder, block.text(), within)
    return [...holder.childNodes]
  }

  function sync(): void {
    const root = options.root()
    if (!root)
      return
    const list = blocks.value
    const steps = options.steps()
    let cursor = root.firstChild
    for (let index = 0; index < list.length;) {
      const current = list[index]!
      if (showsRender(current, cursor)) {
        const last = current.shown!.nodes.at(-1)
        if (last)
          cursor = last.nextSibling
        index += 1
        continue
      }
      // The run of blocks to render, up to the next that shows its render.
      let end = index + 1
      while (end < list.length && !showsRender(list[end]!))
        end += 1
      const before = list.slice(end).find((later) => showsRender(later) && later.shown!.nodes.length > 0)?.shown!.nodes[0] ?? null
      const old: ChildNode[] = []
      for (let node = cursor; node && node !== before; node = node.nextSibling)
        old.push(node)
      const fresh = list.slice(index, end).map((rendering) => render(rendering, steps))
      const placed = place(root, old, fresh.flat(), before)
      let at = 0
      fresh.forEach((nodes, offset) => {
        const shown = placed.slice(at, at + nodes.length)
        at += nodes.length
        const added = shown.filter((node, n) => node === nodes[n])
        const rendered = list[index + offset]!
        rendered.shown = { html: rendered.html.value, nodes: shown, found: options.describe(shown, added) }
      })
      cursor = before
      index = end
    }
    // What follows the last block is left of blocks that have gone.
    for (let node = cursor; node;) {
      const next = node.nextSibling
      node.remove()
      node = next
    }
    options.rendered(list.map((entry) => entry.shown!.found))
  }

  watch([options.text, options.streaming], update, { immediate: true })
  watch([options.root, () => blocks.value.map((entry) => entry.html.value)], sync, { flush: 'post', immediate: true })
  onScopeDispose(() => replace([]))
}

/**
 * Puts `fresh` in `parent` before `before`, in place of `old`, keeping each
 * old node that stands where a fresh one renders the same, and answers the
 * nodes placed.
 */
function place(parent: Node, old: readonly ChildNode[], fresh: readonly ChildNode[], before: ChildNode | null): ChildNode[] {
  const placed = fresh.map((node, index) => {
    const was = old[index]
    const as = renderedAs.get(node)
    return was !== undefined && as !== undefined && renderedAs.get(was) === as ? was : node
  })
  const keep = new Set(placed)
  for (const node of old) {
    if (!keep.has(node))
      node.remove()
  }
  let next = before
  for (let index = placed.length - 1; index >= 0; index -= 1) {
    const node = placed[index]!
    if (node.parentNode !== parent || node.nextSibling !== next)
      parent.insertBefore(node, next)
    next = node
  }
  return placed
}
