<script setup lang="ts">
import { computed, nextTick, ref, shallowRef, watch } from 'vue'
import { renderMarkdownDocument, type DocumentPlace } from '../markdown/document'
import { useMarkdownRenderVersion } from '../markdown/highlight'
import { useContentScrollers } from '../composables/useContentScrollers'
import ScrollArea from '../ui/ScrollArea.vue'
import CodeBlockCopy from '../markdown/CodeBlockCopy.vue'

/**
 * A Markdown file rendered as a document (`file-previews.md` § Markdown). A
 * link to a file asks the host to open it, a `#` link scrolls the document
 * to its heading or anchor, and a web link opens in a new tab of the web browser.
 */
const props = defineProps<{
  text: string
  place: DocumentPlace
  /** Counts the times the document was read again; each time, the images that did not load try again. */
  reloads?: number
}>()
const emit = defineEmits<{ open: [path: string] }>()

const renderVersion = useMarkdownRenderVersion()
const html = computed(() => {
  // The highlighter arriving, or the theme changing, renders the code anew.
  void renderVersion.value
  return renderMarkdownDocument(props.text, props.place)
})
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

/**
 * An image whose bytes do not load, as while the Host is away, shows its alt
 * text in a quiet box in its place, as a web browser shows a missing image's
 * description, instead of its broken picture glyph. Each placeholder keeps
 * its image, which tries again when the document is read again.
 */
const failedImages = new Map<HTMLElement, HTMLImageElement>()

function onImageError(event: Event): void {
  const image = event.target
  if (!(image instanceof HTMLImageElement))
    return
  const placeholder = document.createElement('span')
  placeholder.className = 'inline-flex max-w-full items-center rounded-md border border-dashed border-line px-2 py-0.5 align-middle text-[12px] text-fg-muted'
  const words = image.alt.trim() || 'Could not show this image.'
  placeholder.setAttribute('role', 'img')
  placeholder.setAttribute('aria-label', words)
  placeholder.textContent = words
  failedImages.set(placeholder, image)
  image.replaceWith(placeholder)
}

// A new rendering brings its own images.
watch(html, () => failedImages.clear())
watch(() => props.reloads, () => {
  for (const [placeholder, image] of failedImages) {
    placeholder.replaceWith(image)
    // Setting the address again loads it again.
    image.src = image.src
  }
  failedImages.clear()
})
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
