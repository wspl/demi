<script setup lang="ts">
import { computed, nextTick, onMounted, ref, shallowRef, watch } from 'vue'
import { md } from '@demicodes/web-ui/markdown/md'
import { holdUndecidedMedium } from '@demicodes/web-ui/markdown/render'
import { fitMessageMedia, fitMessageMedium } from '@demicodes/web-ui/markdown/media-run'
import { useMarkdownRenderVersion } from '@demicodes/web-ui/markdown/highlight'
import CodeBlockCopy from '@demicodes/web-ui/markdown/CodeBlockCopy.vue'
import { openMessageLink, useMessageFiles } from '@demicodes/web-ui/markdown/message-files'
import { useMediaViewer } from '@demicodes/web-ui/files/media-viewer'
import { useContentScrollers } from '@demicodes/web-ui/composables/useContentScrollers'
import { useStreamReveal, type RevealStep } from '@demicodes/web-ui/composables/useStreamReveal'
import {
  closeOpenInlineMarkdown,
  holdIncompleteMarkdown,
  renderedLength,
  startOfLast,
  STREAM_PACE,
} from '@demicodes/web-ui/ui/stream-reveal'

// The root is the rendered message; each code block's Copy is teleported into it.
defineOptions({ inheritAttrs: false })

const props = withDefaults(defineProps<{
  content: string
  streaming?: boolean
}>(), {
  streaming: false,
})

const root = ref<HTMLElement>()
useContentScrollers(root)
/** The part of a text that renders as it will stay: no markdown left half open, no medium still undecided. */
function settled(text: string): string {
  return holdUndecidedMedium(holdIncompleteMarkdown(text).visible)
}
const { shown, steps } = useStreamReveal(() => props.content, () => props.streaming, settled)

// While the text arrives the steps show only what has settled; the stream's
// end shows the rest as it is.
const visible = computed(() => (props.streaming ? settled(shown.value) : shown.value))

const renderVersion = useMarkdownRenderVersion()
const files = useMessageFiles()
const viewer = useMediaViewer()

// Closers apply to finished text too: an aborted stream leaves the same half-open markers.
// The render also follows the highlighter: its arrival and the document's theme.
const renderedMarkdown = computed(() => {
  void renderVersion.value
  return md.render(visible.value + closeOpenInlineMarkdown(visible.value), { files: files() })
})

/**
 * Gives the text of each step still fading in a span that fades it, from the
 * render's end backwards. A render replaces the spans, so each starts its
 * fade as far in as the step's own has run, and the fade goes on unbroken.
 * A step's extent is its rendered length, counted from the end; code keeps
 * its place in the count but no span, as a code block's text shows at once.
 */
function wrapSteps(el: HTMLElement, text: string, shownSteps: readonly RevealStep[]): void {
  const now = performance.now()
  const marks = shownSteps
    .filter((step) => now - step.at < STREAM_PACE.fadeMs && step.start < text.length)
    .toSorted((a, b) => b.start - a.start)
    .map((step) => ({ fromEnd: renderedLength(text.slice(step.start)), delay: step.at - now }))
  if (marks.length === 0)
    return
  let counted = 0
  let mark = 0
  const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT)
  for (
    let node = walker.lastChild() as Text | null;
    node && mark < marks.length;
    node = walker.previousNode() as Text | null
  ) {
    const content = node.textContent ?? ''
    // Whitespace between blocks (the newline marked emits after each paragraph) carries no
    // ink, and the count leaves out the newlines between blocks.
    if (!content.trim())
      continue
    const code = node.parentElement?.closest('pre') !== null
    // `end` and `left` walk the node's text from its end; line breaks take no part in the count.
    let end = content.length
    let left = content.length - (content.match(/\n/g)?.length ?? 0)
    while (left > 0 && mark < marks.length) {
      const { fromEnd, delay } = marks[mark]!
      const take = Math.min(fromEnd - counted, left)
      const start = startOfLast(content.slice(0, end), take)
      if (take > 0 && !code) {
        // A span of inline code shown whole in one step fades with its fill, never as an empty box first.
        const parent = node.parentElement
        const inked = start === 0 && end === content.length && parent?.tagName === 'CODE' && parent.childNodes.length === 1
          ? parent
          : document.createElement('span')
        if (inked !== parent) {
          const piece = node.splitText(start)
          piece.replaceWith(inked)
          inked.append(piece)
        }
        inked.classList.add('stream-ink')
        inked.style.animationDuration = `${STREAM_PACE.fadeMs}ms`
        inked.style.animationDelay = `${delay}ms`
      }
      end = start
      left -= take
      counted += take
      if (counted >= fromEnd)
        mark += 1
    }
  }
}

/**
 * The code blocks of the current render, each of which gets its Copy. A
 * render replaces them; the Copy of each place moves into the new one and
 * keeps its Copied.
 */
const codeBlocks = shallowRef<HTMLElement[]>([])

function findCodeBlocks(el: HTMLElement): void {
  codeBlocks.value = [...el.querySelectorAll<HTMLElement>('.code-block')]
}

// A render replaces the images and videos, which take their box before they
// paint; one still loading takes it again when its size arrives.
onMounted(() => {
  if (root.value) {
    fitMessageMedia(root.value)
    findCodeBlocks(root.value)
  }
})

function onMediumLoad(event: Event): void {
  if (event.target instanceof HTMLImageElement || event.target instanceof HTMLVideoElement)
    fitMessageMedium(event.target)
}

watch(
  [renderedMarkdown, () => props.streaming],
  async () => {
    await nextTick()
    const el = root.value
    if (!el)
      return
    fitMessageMedia(el)
    findCodeBlocks(el)
    wrapSteps(el, visible.value, steps.value)
  },
  { flush: 'post' },
)
</script>

<template>
  <div
    v-bind="$attrs"
    ref="root"
    class="markdown-body select-text"
    :class="streaming ? 'is-streaming' : ''"
    :aria-busy="streaming || undefined"
    v-html="renderedMarkdown"
    @click="openMessageLink($event, files(), viewer)"
    @load.capture="onMediumLoad"
    @loadedmetadata.capture="onMediumLoad"
  />
  <Teleport v-for="(block, index) in codeBlocks" :key="index" :to="block">
    <CodeBlockCopy :block="block" />
  </Teleport>
</template>
