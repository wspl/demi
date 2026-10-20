<script setup lang="ts">
import { computed, nextTick, ref, shallowRef, watch } from 'vue'
import { renderMarkdownDocument, type DocumentPlace } from '../markdown/document'
import { useDocumentImages } from './document-images'
import type { FileContents } from './types'
import { useMarkdownRenderVersion } from '../markdown/render-version'
import { useTheme } from '../theme/appTheme'
import { useContentScrollers } from '../composables/useContentScrollers'
import ScrollArea from '../ui/ScrollArea.vue'
import CodeBlockCopy from '../markdown/CodeBlockCopy.vue'

/**
 * A Markdown file rendered as a document. A link to a file asks the host to open
 * it, a `#` link scrolls the document to its heading or anchor, and a web link
 * opens in a new tab of the web browser.
 */
const props = defineProps<{
  text: string
  place: DocumentPlace
  /** The Host's files, which the document's images show as the Host has them now. */
  contents?: FileContents
  /** Counts the times the document was read again; each time, the images that did not load try again. */
  reloads?: number
}>()
const emit = defineEmits<{ open: [path: string] }>()

const renderVersion = useMarkdownRenderVersion()
const { theme } = useTheme()
const rendered = computed(() => {
  // The highlighter or KaTeX arriving, or the theme changing, renders the code and math anew.
  void renderVersion.value
  // The theme also picks the images made for it.
  return renderMarkdownDocument(props.text, props.place, theme.value)
})
const html = computed(() => rendered.value.html)
const scrollArea = ref<InstanceType<typeof ScrollArea>>()
const root = computed(() => scrollArea.value?.el)
useContentScrollers(root)

// Each code block's Copy, as a message's has, in the frame the renderer gives it.
const codeBlocks = shallowRef<HTMLElement[]>([])
watch(html, async () => {
  await nextTick()
  codeBlocks.value = [...root.value?.querySelectorAll<HTMLElement>('.code-block') ?? []]
}, { immediate: true, flush: 'post' })

function follow(event: MouseEvent): void {
  const link = event.target instanceof Element ? event.target.closest('a') : null
  if (!link || !root.value?.contains(link))
    return
  const href = link.getAttribute('href') ?? ''
  if (href.startsWith('#')) {
    event.preventDefault()
    // A heading's id, or an older document's `<a name>` anchor. Only the
    // document scrolls; the page around it stays where it is.
    const anchor = CSS.escape(href.slice(1))
    const target = root.value.querySelector(`[id="${anchor}"], a[name="${anchor}"]`)
    if (target) {
      const offset = target.getBoundingClientRect().top - root.value.getBoundingClientRect().top
      root.value.scrollTo({ top: root.value.scrollTop + offset })
    }
    return
  }
  if (link.hasAttribute('data-file-link')) {
    event.preventDefault()
    emit('open', href)
  }
}

const { onError: onImageError, reload } = useDocumentImages({
  root: () => root.value,
  html: () => html.value,
  hostFiles: () => rendered.value.hostFiles,
  contents: () => props.contents,
})
watch(() => props.reloads, reload)
</script>

<template>
  <!-- The page a reply reads on: the transcript's surface, so a document and a reply look alike. -->
  <ScrollArea ref="scrollArea" class="h-full bg-surface" @click="follow">
    <!-- eslint-disable-next-line vue/no-v-html -- sanitized by the document renderer -->
    <article
      class="markdown-body markdown-document mx-auto max-w-[860px] select-text px-8 py-6 text-conversation text-fg-body"
      @error.capture="onImageError"
      v-html="html"
    />
  </ScrollArea>
  <Teleport v-for="(block, index) in codeBlocks" :key="index" :to="block">
    <CodeBlockCopy :block="block" />
  </Teleport>
</template>
