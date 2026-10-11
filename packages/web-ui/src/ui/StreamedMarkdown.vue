<script setup lang="ts">
import { computed, nextTick, onMounted, ref, shallowRef, watch } from 'vue'
import { useElementSize } from '@vueuse/core'
import { md } from '@demicodes/web-ui/markdown/md'
import { fitMessageMedia, fitMessageMedium, messagePicture, messagePictures, type MessagePicture } from '@demicodes/web-ui/markdown/media-run'
import { useMarkdownRenderVersion } from '@demicodes/web-ui/markdown/highlight'
import CodeBlockCopy from '@demicodes/web-ui/markdown/CodeBlockCopy.vue'
import { openMessageLink, pressMessageLink, useMessageFiles } from '@demicodes/web-ui/markdown/message-files'
import { replyCodeFiles } from '@demicodes/web-ui/markdown/code-files'
import { useMessageAgents } from '@demicodes/web-ui/agent/agent-links'
import AgentLinkTip from '@demicodes/web-ui/agent/AgentLinkTip.vue'
import { attachmentDrag } from '@demicodes/web-ui/agent/message-input/copied-message'
import PicturePreview from '@demicodes/web-ui/ui/PicturePreview.vue'
import FileIcon from '@demicodes/web-ui/files/FileIcon.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import { useMediaViewer } from '@demicodes/web-ui/files/media-viewer'
import { useContentScrollers } from '@demicodes/web-ui/composables/useContentScrollers'
import { useStreamReveal, type RevealStep } from '@demicodes/web-ui/composables/useStreamReveal'
import {
  closeOpenInlineMarkdown,
  holdIncompleteMarkdown,
  prefersReducedMotion,
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
const files = useMessageFiles()
/** The part of a text that renders as it will stay: no markdown left half open. */
function settled(text: string): string {
  return holdIncompleteMarkdown(text).visible
}
const { shown, steps } = useStreamReveal(() => props.content, () => props.streaming, settled)

// While the text arrives the steps show only what has settled; the stream's
// end shows the rest as it is.
const visible = computed(() => (props.streaming ? settled(shown.value) : shown.value))

const renderVersion = useMarkdownRenderVersion()
const agents = useMessageAgents()
const viewer = useMediaViewer()
// The files the reply's inline code names, asked about once it is complete.
const codeFiles = computed(() => replyCodeFiles(props.content, props.streaming, files()))

// Closers apply to finished text too: an aborted stream leaves the same half-open markers.
// The render also follows the highlighter: its arrival and the document's theme.
const renderedMarkdown = computed(() => {
  void renderVersion.value
  return md.render(visible.value + closeOpenInlineMarkdown(visible.value), {
    files: files(),
    codeFiles: codeFiles.value,
    agents: agents(),
  })
})

/** How a step's text fades in. */
const FADE_IN: Keyframe[] = [{ opacity: 0 }, { opacity: 1 }]

/**
 * Gives the text of each step still fading in a span that fades it, from the
 * render's end backwards. A step's extent is its rendered length, counted
 * from the end; code keeps its place in the count but no span, as a code
 * block's text shows at once.
 *
 * Each fade is an animation of the document's timeline that starts when its
 * step was shown (`performance.now()` and the timeline count from the same
 * origin), so it goes on unbroken through a render, which replaces the spans,
 * and through a move of the text in the page, which a CSS animation would
 * start over: setting up a scroller's bars (`ScrollArea`) appends its
 * content to it again, and the reply's last lines faded in a second time.
 */
function wrapSteps(el: HTMLElement, text: string, shownSteps: readonly RevealStep[]): void {
  if (prefersReducedMotion())
    return
  const now = performance.now()
  const marks = shownSteps
    .filter((step) => now - step.at < STREAM_PACE.fadeMs && step.start < text.length)
    .toSorted((a, b) => b.start - a.start)
    .map((step) => ({ fromEnd: renderedLength(text.slice(step.start)), at: step.at }))
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
      const { fromEnd, at } = marks[mark]!
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
        // The class marks the text still fading in, for whoever looks at the page.
        inked.classList.add('stream-ink')
        inked.animate(FADE_IN, { duration: STREAM_PACE.fadeMs }).startTime = at
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
 * The places of the current render that components fill: each code block
 * gets its Copy, each file link its file's icon, and each agent link its
 * agent's mark and tip. A render replaces them; the component of each place
 * moves into the new one and keeps its state, as a code block's Copied.
 */
const codeBlocks = shallowRef<HTMLElement[]>([])
const fileIcons = shallowRef<HTMLElement[]>([])
const agentLinks = shallowRef<HTMLElement[]>([])
/** The message's pictures, each a tile with its preview on hover. */
const pictures = shallowRef<MessagePicture[]>([])

function findPlaces(el: HTMLElement): void {
  codeBlocks.value = [...el.querySelectorAll<HTMLElement>('.code-block')]
  fileIcons.value = [...el.querySelectorAll<HTMLElement>('[data-file-icon]')]
  agentLinks.value = [...el.querySelectorAll<HTMLElement>('[data-agent-link]')]
  pictures.value = messagePictures(el)
}

// A render replaces the images and videos, which take their tile before they
// paint; one still loading takes it again when its size arrives, and every
// one again when the message's width changes.
const { width } = useElementSize(root)
watch(width, () => {
  if (root.value)
    fitMessageMedia(root.value)
})

onMounted(() => {
  if (root.value) {
    fitMessageMedia(root.value)
    findPlaces(root.value)
  }
})

function onMediumLoad(event: Event): void {
  const medium = event.target
  if (!(medium instanceof HTMLImageElement || medium instanceof HTMLVideoElement))
    return
  fitMessageMedium(medium)
  // A picture whose size its render did not know has it now, for its preview's box.
  if (pictures.value.some((picture) => picture.element === medium))
    pictures.value = pictures.value.map((picture) => (picture.element === medium ? messagePicture(medium) : picture))
}

watch(
  [renderedMarkdown, () => props.streaming],
  async () => {
    await nextTick()
    const el = root.value
    if (!el)
      return
    fitMessageMedia(el)
    findPlaces(el)
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
    @click="openMessageLink($event, files(), viewer, agents())"
    @dragstart="attachmentDrag"
    @keydown="pressMessageLink"
    @load.capture="onMediumLoad"
    @loadedmetadata.capture="onMediumLoad"
  />
  <Teleport v-for="(block, index) in codeBlocks" :key="index" :to="block">
    <CodeBlockCopy :block="block" />
  </Teleport>
  <Teleport v-for="(place, index) in fileIcons" :key="index" :to="place">
    <FileIcon :name="place.dataset.fileIcon ?? ''" :is-directory="false" :size="ICON_PX.in28" />
  </Teleport>
  <PicturePreview
    v-for="(picture, index) in pictures"
    :key="index"
    :trigger="picture.element"
    :kind="picture.kind"
    :src="picture.src"
    :size="picture.size"
    :caption="picture.caption"
  />
  <AgentLinkTip
    v-for="(link, index) in agentLinks"
    :key="index"
    :element="link"
    :number="Number(link.dataset.agentLink)"
  />
</template>
