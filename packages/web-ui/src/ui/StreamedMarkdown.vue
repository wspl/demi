<script setup lang="ts">
import { computed, onScopeDispose, ref, shallowReactive, shallowRef, watch, type ComputedRef } from 'vue'
import { useElementSize } from '@vueuse/core'
import { md } from '@demicodes/web-ui/markdown/md'
import { fitMessageMedia, fitMessageMedium, messagePicture, messagePictures, type MessagePicture } from '@demicodes/web-ui/markdown/media-run'
import { useMarkdownRenderVersion } from '@demicodes/web-ui/markdown/render-version'
import CodeBlockCopy from '@demicodes/web-ui/markdown/CodeBlockCopy.vue'
import { messageLinkMenu, openMessageLink, pressMessageLink, useMessageFiles, useRenderedFiles } from '@demicodes/web-ui/markdown/message-files'
import { vLinkMenus, type LinkMenus } from '@demicodes/web-ui/ui/context-menu'
import { replyCodeFiles } from '@demicodes/web-ui/markdown/code-files'
import { hostImagePaths, useHostImagesAt } from '@demicodes/web-ui/markdown/host-images'
import type { HostImage } from '@demicodes/web-ui/files/host-image'
import type { MessageAttachment } from '@demicodes/web-ui/markdown/types'
import { useMessageAgents } from '@demicodes/web-ui/agent/agent-links'
import { useTranscript } from '@demicodes/web-ui/agent/edit-selection'
import AgentLinkMark from '@demicodes/web-ui/agent/AgentLinkMark.vue'
import { attachmentDrag } from '@demicodes/web-ui/agent/message-input/copied-message'
import PicturePreview from '@demicodes/web-ui/ui/PicturePreview.vue'
import { GitCommitHorizontal, Link } from '@lucide/vue'
import FileIcon from '@demicodes/web-ui/files/FileIcon.vue'
import AudioPlayer from '@demicodes/web-ui/files/AudioPlayer.vue'
import AttachmentCard from '@demicodes/web-ui/files/AttachmentCard.vue'
import { ATTACHMENT_AUDIO_ATTRIBUTE, ATTACHMENT_CARD_ATTRIBUTE, messageAttachment } from '@demicodes/web-ui/markdown/render'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import { useMediaViewer } from '@demicodes/web-ui/files/media-viewer'
import { useContentScrollers } from '@demicodes/web-ui/composables/useContentScrollers'
import type { TextArrival } from '@demicodes/conversation-client'
import { useStreamReveal } from '@demicodes/web-ui/composables/useStreamReveal'
import { useMarkdownBlocks } from '@demicodes/web-ui/composables/useMarkdownBlocks'
import { holdIncompleteMarkdown } from '@demicodes/web-ui/ui/stream-reveal'

// The root is the rendered message; each code block's Copy is teleported into it.
defineOptions({ inheritAttrs: false })

const props = withDefaults(defineProps<{
  content: string
  streaming?: boolean
  /** When each piece of `content` reached the page, so a view that mounts mid-stream fades each from then. */
  arrivals?: readonly TextArrival[]
}>(), {
  streaming: false,
  arrivals: undefined,
})

const root = ref<HTMLElement>()
useContentScrollers(root)
const files = useMessageFiles()
/** The part of a text that renders as it will stay: no markdown left half open. */
function settled(text: string): string {
  return holdIncompleteMarkdown(text).visible
}
const { shown, steps } = useStreamReveal(() => props.content, () => props.streaming, settled, () => props.arrivals)

// While the text arrives the steps show only what has settled; the stream's
// end shows the rest as it is.
const visible = computed(() => (props.streaming ? settled(shown.value) : shown.value))

const renderVersion = useMarkdownRenderVersion()
const agents = useMessageAgents()
const viewer = useMediaViewer()
const transcript = useTranscript()
const renderFiles = useRenderedFiles(files)
/** A link to a file, a commit or an agent has that object's menu, as the conversation has it when the menu opens. */
const linkMenus: LinkMenus = (link) => messageLinkMenu(link, files(), agents())
// The files the reply's inline code names, asked about once it is complete.
const codeFiles = computed(() => replyCodeFiles(props.content, props.streaming, renderFiles.value))

/** The Host image paths each block that shows names. */
const blockImagePaths = shallowReactive(new Set<ComputedRef<string[]>>())
// The Host images of what shows, each at the version of its file the page keeps.
const hostImages = useHostImagesAt(() => renderFiles.value?.contents, () => [...blockImagePaths].flatMap((paths) => paths.value))

/** Whether two blocks' Host images show the same. */
function sameImages(a: ReadonlyMap<string, HostImage>, b: ReadonlyMap<string, HostImage>): boolean {
  if (a.size !== b.size)
    return false
  for (const [path, image] of a) {
    const other = b.get(path)
    if (other?.state !== image.state || (other.state === 'shown' && image.state === 'shown' && other.src !== image.src))
      return false
  }
  return true
}

/**
 * A block's render, which follows the highlighter (its arrival and the document's
 * theme) and what it names: its Host images, the files of its code, its agents.
 *
 * It reads its own Host images, so a change of another block's leaves it as it is.
 */
function renderBlock(source: () => string): () => string {
  const paths = computed(() => hostImagePaths(source(), renderFiles.value))
  blockImagePaths.add(paths)
  onScopeDispose(() => blockImagePaths.delete(paths))
  const images = computed<ReadonlyMap<string, HostImage>>((previous) => {
    const all = hostImages.value
    const own = new Map(paths.value.flatMap((path) => {
      const image = all.get(path)
      return image ? [[path, image] as const] : []
    }))
    return previous && sameImages(previous, own) ? previous : own
  })
  return () => {
    void renderVersion.value
    return md.render(source(), {
      files: renderFiles.value,
      codeFiles: codeFiles.value,
      hostImages: images.value,
      agents: agents(),
    })
  }
}

/**
 * The places of a render that components fill:
 *
 * - Each code block gets its Copy.
 * - Each link gets its icon (a file's, a commit's or a web page's).
 * - Each attachment block that is a card gets its card.
 * - Each agent link gets its agent's mark and tip.
 *
 * A render replaces them; the component of each place moves into the new one and
 * keeps its state, as a code block's Copied.
 */
interface Places {
  codeBlocks: HTMLElement[]
  linkIcons: HTMLElement[]
  agentLinks: HTMLElement[]
  /** The places of audio attachments' inline players. */
  audios: HTMLElement[]
  /** The places of attachment blocks' cards. */
  cards: HTMLElement[]
  media: (HTMLImageElement | HTMLVideoElement)[]
}
const codeBlocks = shallowRef<HTMLElement[]>([])
const linkIcons = shallowRef<HTMLElement[]>([])
const agentLinks = shallowRef<HTMLElement[]>([])
const audios = shallowRef<HTMLElement[]>([])
const cards = shallowRef<HTMLElement[]>([])
/** The message's pictures, each a tile with its preview on hover. */
const pictures = shallowRef<MessagePicture[]>([])
/** Each medium's picture, kept while its render shows, so its loaded size stays with it. */
const pictureOf = new WeakMap<Element, MessagePicture>()

/** The places among a block's nodes; the media of `fresh`, new to the page, take their tiles first. */
function placesIn(nodes: readonly ChildNode[], fresh: readonly ChildNode[]): Places {
  const elements = nodes.filter((node) => node instanceof HTMLElement)
  for (const node of fresh) {
    if (node instanceof HTMLElement)
      fitMessageMedia(node)
  }
  const within = (selector: string) => elements.flatMap((element) => [
    ...(element.matches(selector) ? [element] : []),
    ...element.querySelectorAll<HTMLElement>(selector),
  ])
  return {
    codeBlocks: within('.code-block'),
    linkIcons: within('[data-file-icon], [data-commit-icon], [data-link-icon]'),
    agentLinks: within('[data-agent-link]'),
    audios: within(`[${ATTACHMENT_AUDIO_ATTRIBUTE}]`),
    cards: within(`[${ATTACHMENT_CARD_ATTRIBUTE}]`),
    media: elements.flatMap((element) => messagePictures(element).map((picture) => picture.element)),
  }
}

useMarkdownBlocks({
  root: () => root.value,
  text: () => visible.value,
  streaming: () => props.streaming,
  steps: () => steps.value,
  render: renderBlock,
  describe: placesIn,
  rendered(found) {
    codeBlocks.value = found.flatMap((places) => places.codeBlocks)
    linkIcons.value = found.flatMap((places) => places.linkIcons)
    agentLinks.value = found.flatMap((places) => places.agentLinks)
    audios.value = found.flatMap((places) => places.audios)
    cards.value = found.flatMap((places) => places.cards)
    pictures.value = found.flatMap((places) => places.media).map((medium) => {
      const picture = pictureOf.get(medium) ?? messagePicture(medium)
      pictureOf.set(medium, picture)
      return picture
    })
  },
})

// A render's images and videos take their tile before they paint; one still
// loading takes it again when its size arrives, and every one again when the
// message's width changes.
const { width } = useElementSize(root)
watch(width, () => {
  if (root.value)
    fitMessageMedia(root.value)
})

/** The attachment a card's place names, as the conversation has it now; null while the page asks for it. */
function cardAttachment(place: HTMLElement): { id: string; attachment: MessageAttachment | null } {
  const id = place.getAttribute(ATTACHMENT_CARD_ATTRIBUTE) ?? ''
  const lookup = messageAttachment(files(), id)
  return { id, attachment: lookup?.state === 'found' ? lookup.attachment : null }
}

/**
 * A card's click opens the file in the work panel: the file it was uploaded
 * from, as it is now, while that file can be read, and its copy otherwise.
 */
async function openCard(place: HTMLElement): Promise<void> {
  const { id, attachment } = cardAttachment(place)
  const shown = files()
  if (!attachment || !shown)
    return
  // A question that fails is answered as a file that is not there: the copy opens.
  const origin = shown.open && shown.attachmentOrigin ? await shown.attachmentOrigin(id, attachment).catch(() => null) : null
  if (origin && shown.open)
    shown.open({ path: origin, line: null })
  else
    shown.openAttachment?.(id, attachment)
}

function onMediumLoad(event: Event): void {
  const medium = event.target
  if (!(medium instanceof HTMLImageElement || medium instanceof HTMLVideoElement))
    return
  fitMessageMedium(medium)
  // A picture whose size its render did not know has it now, for its preview's box.
  if (pictureOf.has(medium)) {
    const picture = messagePicture(medium)
    pictureOf.set(medium, picture)
    pictures.value = pictures.value.map((shown) => (shown.element === medium ? picture : shown))
  }
}
</script>

<template>
  <div
    v-bind="$attrs"
    ref="root"
    v-link-menus="linkMenus"
    class="markdown-body select-text"
    :class="streaming ? 'is-streaming' : ''"
    :aria-busy="streaming || undefined"
    @click="openMessageLink($event, files(), viewer, agents(), transcript?.pictures)"
    @dragstart="attachmentDrag"
    @keydown="pressMessageLink"
    @load.capture="onMediumLoad"
    @loadedmetadata.capture="onMediumLoad"
  />
  <Teleport v-for="(block, index) in codeBlocks" :key="index" :to="block">
    <CodeBlockCopy :block="block" />
  </Teleport>
  <Teleport v-for="(place, index) in linkIcons" :key="index" :to="place">
    <FileIcon v-if="place.dataset.fileIcon !== undefined" :name="place.dataset.fileIcon" :is-directory="false" :size="ICON_PX.in28" />
    <GitCommitHorizontal v-else-if="place.dataset.commitIcon !== undefined" :size="ICON_PX.in28" aria-hidden="true" />
    <Link v-else :size="ICON_PX.in28" aria-hidden="true" />
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
  <Teleport v-for="(place, index) in audios" :key="index" :to="place">
    <AudioPlayer :src="place.getAttribute(ATTACHMENT_AUDIO_ATTRIBUTE) ?? ''" :name="place.dataset.audioName ?? ''" />
  </Teleport>
  <Teleport v-for="(place, index) in cards" :key="index" :to="place">
    <AttachmentCard
      :file="cardAttachment(place).attachment"
      :openable="files()?.openAttachment !== undefined"
      @open="void openCard(place)"
    />
  </Teleport>
  <AgentLinkMark
    v-for="(link, index) in agentLinks"
    :key="index"
    :element="link"
    :number="Number(link.dataset.agentLink)"
  />
</template>
