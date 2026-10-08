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
import { useStreamReveal } from '@demicodes/web-ui/composables/useStreamReveal'
import {
  closeOpenInlineMarkdown,
  holdIncompleteMarkdown,
  visibleFrontierLength
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
const { shown, frontier } = useStreamReveal(() => props.content, () => props.streaming)

// The text is still arriving while the view catches up with it, which goes on
// for a moment after the stream ends; what it cannot show yet waits until then.
const visible = computed(() => {
  if (!props.streaming && shown.value === props.content)
    return shown.value
  return holdUndecidedMedium(holdIncompleteMarkdown(shown.value).visible)
})

const renderVersion = useMarkdownRenderVersion()
const files = useMessageFiles()
const viewer = useMediaViewer()

// Closers apply to finished text too: an aborted stream leaves the same half-open markers.
// The render also follows the highlighter: its arrival and the document's theme.
const renderedMarkdown = computed(() => {
  void renderVersion.value
  return md.render(visible.value + closeOpenInlineMarkdown(visible.value), { files: files() })
})

/** The frontier spans of the current render, so clearing them is not a subtree search. */
let inkSpans: HTMLSpanElement[] = []

// Only the last few characters are the frontier: walk text nodes backwards from the end and
// stop as soon as the budget is spent instead of collecting every text node of the block.
function wrapFrontier(el: HTMLElement, charCount: number): void {
  inkSpans = []
  if (charCount <= 0)
    return
  const last = el.lastElementChild
  if (last?.matches('.code-block, .table-scroll'))
    return

  let remaining = charCount
  const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT)
  for (
    let node = walker.lastChild() as Text | null;
    node && remaining > 0;
    node = walker.previousNode() as Text | null
  ) {
    const text = node.textContent ?? ''
    // Whitespace between blocks (the newline marked emits after each paragraph) carries no
    // ink; wrapping it makes an extra line that vanishes when the marks clear.
    if (!text.trim() || node.parentElement?.closest('pre'))
      continue
    const take = Math.min(remaining, text.length)
    const rest = node.splitText(text.length - take)
    const span = document.createElement('span')
    span.className = 'stream-ink'
    rest.parentNode?.insertBefore(span, rest)
    span.appendChild(rest)
    inkSpans.push(span)
    remaining -= take
  }
}

function clearStreamMarks(): void {
  for (const span of inkSpans) {
    if (span.isConnected)
      span.replaceWith(...span.childNodes)
  }
  inkSpans = []
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
    if (!props.streaming) {
      clearStreamMarks()
      return
    }
    wrapFrontier(el, visibleFrontierLength(visible.value, frontier.value))
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
