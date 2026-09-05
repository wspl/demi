<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { md } from '@demicodes/web-ui/markdown/md'
import { isHttpUrl } from '@demicodes/web-ui/markdown/filePath'
import { useStreamReveal } from '@demicodes/web-ui/composables/useStreamReveal'
import { holdIncompleteMarkdown, visibleFrontierLength } from '@demicodes/web-ui/ui/stream-reveal'

const props = withDefaults(defineProps<{
  content: string
  streaming?: boolean
}>(), {
  streaming: false,
})

const root = ref<HTMLElement>()
const { shown, frontier } = useStreamReveal(() => props.content, () => props.streaming)

const visible = computed(() => {
  if (!props.streaming) return shown.value
  return holdIncompleteMarkdown(shown.value).visible
})

const renderedMarkdown = computed(() => md.render(visible.value))

/** The frontier spans of the current render, so clearing them is not a subtree search. */
let inkSpans: HTMLSpanElement[] = []

// Only the last few characters are the frontier: walk text nodes backwards from the end and
// stop as soon as the budget is spent instead of collecting every text node of the block.
function wrapFrontier(el: HTMLElement, charCount: number): void {
  inkSpans = []
  if (charCount <= 0) return
  const last = el.lastElementChild
  if (last && (last.tagName === 'PRE' || last.tagName === 'TABLE')) return

  let remaining = charCount
  const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT)
  for (let node = walker.lastChild() as Text | null; node && remaining > 0; node = walker.previousNode() as Text | null) {
    const text = node.textContent ?? ''
    if (!text || node.parentElement?.closest('pre')) continue
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
    if (span.isConnected) span.replaceWith(...span.childNodes)
  }
  inkSpans = []
}

watch(
  [renderedMarkdown, () => props.streaming],
  async () => {
    await nextTick()
    const el = root.value
    if (!el) return
    if (!props.streaming) {
      clearStreamMarks()
      return
    }
    wrapFrontier(el, visibleFrontierLength(visible.value, frontier.value))
  },
  { flush: 'post' },
)

function handleClick(event: MouseEvent) {
  const target = (event.target as HTMLElement).closest('a')
  if (!target) return
  const href = target.getAttribute('href')
  if (!href) return
  if (isHttpUrl(href)) {
    event.preventDefault()
    window.open(href, '_blank', 'noopener,noreferrer')
  }
}
</script>

<template>
  <div
    ref="root"
    class="markdown-body select-text"
    :class="streaming ? 'is-streaming' : ''"
    :aria-busy="streaming || undefined"
    v-html="renderedMarkdown"
    @click="handleClick"
  />
</template>
