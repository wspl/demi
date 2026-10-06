<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { md } from '@demicodes/web-ui/markdown/md'
import { holdUndecidedMedium } from '@demicodes/web-ui/markdown/render'
import { useMarkdownRenderVersion } from '@demicodes/web-ui/markdown/highlight'
import { openMessageLink, useMessageFiles } from '@demicodes/web-ui/markdown/message-files'
import { useImageViewer } from '@demicodes/web-ui/files/image-viewer'
import { useContentScrollbars } from '@demicodes/web-ui/composables/useContentScrollbars'
import { useStreamReveal } from '@demicodes/web-ui/composables/useStreamReveal'
import {
  closeOpenInlineMarkdown,
  holdIncompleteMarkdown,
  visibleFrontierLength
} from '@demicodes/web-ui/ui/stream-reveal'

const props = withDefaults(defineProps<{
  content: string
  streaming?: boolean
}>(), {
  streaming: false,
})

const root = ref<HTMLElement>()
useContentScrollbars(root)
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
const viewer = useImageViewer()

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
  if (last && (last.tagName === 'PRE' || last.tagName === 'TABLE'))
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

watch(
  [renderedMarkdown, () => props.streaming],
  async () => {
    await nextTick()
    const el = root.value
    if (!el)
      return
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
    ref="root"
    class="markdown-body select-text"
    :class="streaming ? 'is-streaming' : ''"
    :aria-busy="streaming || undefined"
    v-html="renderedMarkdown"
    @click="openMessageLink($event, files(), viewer)"
  />
</template>
