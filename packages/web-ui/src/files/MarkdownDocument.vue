<script setup lang="ts">
import { computed, ref } from 'vue'
import { renderMarkdownDocument, type DocumentPlace } from '../markdown/document'
import { useMarkdownRenderVersion } from '../markdown/highlight'

/**
 * A Markdown file rendered as a document (`file-previews.md` § Markdown). A
 * link to a file asks the host to open it, a `#` link scrolls the document
 * to its heading or anchor, and a web link opens in a new browser tab.
 */
const props = defineProps<{ text: string; place: DocumentPlace }>()
const emit = defineEmits<{ open: [path: string] }>()

const renderVersion = useMarkdownRenderVersion()
const html = computed(() => {
  // The highlighter arriving, or the theme changing, renders the code anew.
  void renderVersion.value
  return renderMarkdownDocument(props.text, props.place)
})
const root = ref<HTMLElement | null>(null)

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
</script>

<template>
  <div ref="root" class="h-full overflow-y-auto" @click="follow">
    <!-- eslint-disable-next-line vue/no-v-html -- sanitized by the document renderer -->
    <article class="markdown-body markdown-document mx-auto max-w-[860px] select-text px-8 py-6 text-conversation text-fg-body" v-html="html" />
  </div>
</template>
