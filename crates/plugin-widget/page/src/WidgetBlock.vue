<script lang="ts">
/**
 * A widget in its reply: its page (`./WidgetFrame.vue`) in a card the block
 * draws, a line with round corners, as wide as the reply's text. The page draws no card of its own, so every
 * widget, a chart or a screen's mockup, stands apart from the reply's text
 * alike; the page fills the card and keeps its own room inside it, so what
 * is meant to reach the edges can.
 *
 * - The frame, inside the card's line, has the size the page declares
 *   (`demi-frame`) from the first moment: the reply's data brings each
 *   widget's frame and title with the reply (`facts`), as it brings the
 *   attachments a reply names, and the block asks nothing of the plugin.
 *   One that declares none starts 320px
 *   high and follows the height the page reports, at most the window's. A
 *   page whose content reports more than the height it declared, as text
 *   wraps at a narrower reply, grows its frame to hold it, once in the
 *   page's life and at most to the window's height: content is not cut, and
 *   a page measured right at the screenshots' width rarely grows. A frame of an
 *   aspect ratio scales with its width, and is as wide as the reply however
 *   its declared bounds pull its height. Whatever the widget shows instead,
 *   its room stays.
 * - A widget the user does not have says so in its room, as does one whose
 *   document its route could not serve, and so does a plugin turned off,
 *   while the block shows or before, in the room the widget had; turned on
 *   again, once the backend has it on, the widget shows again.
 * - Two buttons in the card's corner, always shown since the frame takes the
 *   pointer's hover from the card: Copy as Image puts a PNG of what the frame
 *   shows now on the clipboard (`WidgetFrame`'s `picture`), and Share, where
 *   the page can share (`share`), opens the share dialog for the widget
 *   under its page's title.
 * - `whole` draws the widget alone with no card, as a widget's share shows
 *   it: as wide as its box, as high as in a reply but at most the box's
 *   height, and centered in the box's height when it is lower, the corner's
 *   buttons on the frame's corner.
 */

/**
 * The content's height each widget's page last reported, kept for the page's
 * life, so a frame its content grew keeps its room when the list unmounts
 * its reply and draws it again, or while its plugin is off.
 */
const knownHeights = new Map<string, number>()

/**
 * The widgets whose frame grew past their declared height, which a frame does
 * once in the page's life: a page sized by its viewport reports more each time
 * its frame grows, and would grow it, at each width and each mount, up to the
 * window's height. After that the frame only shrinks back toward it, and
 * content a narrower reply makes taller scrolls inside it.
 */
const grown = new Set<string>()
</script>

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { Check, Images, Share } from '@lucide/vue'
import { IconButton, useCopy, usePage, windowHeight } from '@demicodes/plugin-sdk'
import { widgetFactsSchema, type FrameSize, type WidgetFacts } from './generated/plugin'
import { t } from './locales/text'
import WidgetFrame from './WidgetFrame.vue'

const props = defineProps<{ id: string, facts: unknown, share?: (title: string) => void, whole?: boolean }>()
const page = usePage()

/** The frame's height while a page that declares no size has not reported its own. */
const UNDECLARED_PX = 320

/** The widget, as the reply's data brought it; null for one the user does not have. */
const widget = computed<WidgetFacts | null>(() => {
  const read = widgetFactsSchema.safeParse(props.facts)
  return read.success ? read.data : null
})

/**
 * The widget's title: its page's, else the widget by its number, as an
 * untitled document is named by the app that shows it.
 */
const title = computed(() => widget.value?.title ?? t('block.untitled', { number: props.id.replace(/^w/, '') }))

/** Whether the frame's document could not be served. */
const unavailable = ref(false)

/** What the block shows: the widget's page, or why it does not. */
const state = computed<'shown' | 'missing' | 'off'>(() => {
  if (!page.plugin.enabled.value)
    return 'off'
  return widget.value && !unavailable.value ? 'shown' : 'missing'
})

/** The height the page last reported, for a frame that follows it; it stays while the page draws again. */
const reported = ref<number | null>(knownHeights.get(props.id) ?? null)
watch(() => props.id, (id) => {
  reported.value = knownHeights.get(id) ?? null
})
// A block drawn again, for another widget or with its plugin turned on again, asks for its document again.
watch(() => [props.id, page.plugin.enabled.value] as const, () => {
  unavailable.value = false
})

function onHeight(height: number): void {
  const declaredHeight = declared.value?.height
  if (declaredHeight != null && height > Math.max(reported.value ?? 0, declaredHeight)) {
    if (grown.has(props.id))
      return
    grown.add(props.id)
  }
  reported.value = height
  knownHeights.set(props.id, height)
}

/** The frame the page declares, which the reply's data brought; it stays while the plugin is off. */
const declared = computed<FrameSize | null>(() => widget.value?.frame ?? null)

/** The frame's size, as CSS lays out what the page declares. */
const frameStyle = computed<Record<string, string>>(() => {
  const size = declared.value
  if (size?.height != null || size?.aspectRatio != null) {
    // Content taller than a declared height grows the frame to hold it, up
    // to the window's height and any declared maximum, rather than being cut.
    // A frame of an aspect ratio scales with its width as a drawing does.
    const grows = size.height != null && size.aspectRatio == null && reported.value != null
    const grownTo = grows ? Math.min(reported.value!, windowHeight.value, size.maxHeight ?? Infinity) : 0
    const least = Math.max(size.minHeight ?? 0, grownTo)
    // Whole, a floor would hold the frame past its box, which a flex item
    // cannot shrink below: a frame of a height takes the one it would hold
    // to, and either shrinks to the box from there.
    if (props.whole) {
      return {
        ...(size.height != null ? { height: `${Math.max(size.height, least)}px` } : {}),
        ...(size.aspectRatio != null ? { aspectRatio: `${size.aspectRatio.width} / ${size.aspectRatio.height}` } : {}),
        ...(size.maxHeight != null ? { maxHeight: `${Math.max(size.maxHeight, least)}px` } : {}),
      }
    }
    return {
      ...(size.height != null ? { height: `${size.height}px` } : {}),
      ...(size.aspectRatio != null ? { aspectRatio: `${size.aspectRatio.width} / ${size.aspectRatio.height}` } : {}),
      ...(least > 0 ? { minHeight: `${least}px` } : {}),
      ...(size.maxHeight != null ? { maxHeight: `${Math.max(size.maxHeight, least)}px` } : {}),
    }
  }
  return { height: `${Math.min(reported.value ?? UNDECLARED_PX, windowHeight.value)}px` }
})

/** What the block says in the widget's room when it shows no page. */
const note = computed(() => {
  switch (state.value) {
    case 'off': return t('block.off')
    case 'missing': return t('block.missing', { id: props.id })
    default: return null
  }
})

const frame = ref<InstanceType<typeof WidgetFrame>>()
// The item's picture is still being drawn when it is written: Safari lets a
// page write the clipboard only inside the click.
const { copy, copied } = useCopy(() => frame.value
  ? new ClipboardItem({ 'image/png': frame.value.picture() })
  : undefined)

/** Opens the share dialog for the widget, under its page's title. */
function shareWidget(): void {
  props.share?.(title.value)
}
</script>

<template>
  <!-- Whole, the frame shrinks to the box's height as a flex item may. -->
  <div :class="whole ? 'flex size-full flex-col justify-center' : 'contents'">
    <div class="relative overflow-hidden" :class="whole ? 'flex max-h-full min-h-0 flex-col' : 'rounded-xl border border-line'">
      <div class="w-full" :class="whole ? 'min-h-0 shrink' : ''" :style="frameStyle">
        <WidgetFrame
          v-if="state === 'shown'"
          :id="id"
          ref="frame"
          :title="title"
          @height="onHeight"
          @unavailable="unavailable = true"
        />
      </div>
      <div
        v-if="state === 'shown'"
        class="absolute end-2 top-2 flex gap-0.5 opacity-60 transition-opacity hover:opacity-100 focus-within:opacity-100"
      >
        <IconButton
          :icon="copied ? Check : Images"
          size="sm"
          variant="ghost"
          :aria-label="t('block.copyAsImage')"
          @click="copy"
        />
        <IconButton
          v-if="share"
          :icon="Share"
          size="sm"
          variant="ghost"
          :aria-label="t('block.share')"
          @click="shareWidget"
        />
      </div>
      <p v-if="note" class="absolute inset-0 m-0 flex items-center justify-center text-fg-muted">{{ note }}</p>
    </div>
  </div>
</template>
