<script lang="ts">
/** How long the pointer rests on a picture before its card shows. */
const OPEN_DELAY_MS = 300
/** How soon after a card closed the next picture's shows without the delay. */
const WARM_MS = 300
/** The narrowest a card is, so a small picture's line still reads. */
const CARD_MIN_WIDTH = 240
/** The card's padding, on each side. */
const CARD_PAD = 6
</script>

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import type { Placement } from '@floating-ui/vue'
import { loadedSize, type PixelSize } from '../files/thumbnail'
import { usePhone } from './phone'
import { previewBox, previewLimit, previewPlacement, usePreviewPage } from './picture-preview'
import Tooltip from './Tooltip.vue'

defineOptions({ inheritAttrs: false })

/**
 * A picture's preview on hover (`file-previews.md` § A picture's preview
 * on hover), for every picture the page shows smaller than the card: a
 * row's pictures, a message's pictures, a sent message's image capsule.
 * Resting the pointer on the picture for 300 ms shows a card above it, or
 * below it when the page has no room above, kept within the page, and on
 * the side with more room, scaled down to fit it, when neither side has room
 * (`previewPlacement`), never over the picture: the
 * picture at its own size on the screen, its pixels one to one, scaled down
 * to fit 70% of the page's width and height and never enlarged
 * (`previewBox`), over a checkerboard where it is transparent, a video's
 * first frame, and under it one line, what the picture belongs to, cut at
 * its end, then its pixel size. Its box comes from `size` and the screen's
 * pixel ratio, known before the bytes arrive, so it never grows as they
 * load, and follows the window's resizing and its move to another screen.
 * The card takes no pointer: leaving the picture closes it at
 * once, and a press on the picture, which opens the viewer, and Escape
 * close it. A touch shows none. It is a `Tooltip` card (`card`, `warmMs`,
 * `placeBy`), so it dismisses, stacks and yields to other overlays as every
 * tip does; this component sizes its picture and picks its side.
 *
 * The picture is the slot, or an element the page did not render through a
 * component, `trigger`, such as an image in a rendered message.
 */
const props = defineProps<{
  kind: 'image' | 'video'
  src: string
  /** The picture's pixel size; null while it is unknown, when the card sizes itself as the picture loads. */
  size: PixelSize | null
  /** What the picture belongs to: a step's title, an image's alt text, a file's name. */
  caption: string
  trigger?: HTMLElement | null
  tag?: 'span' | 'div'
}>()

/** The card's image or video. */
const picture = ref<HTMLElement | null>(null)
/** The picture's size once the card's copy loaded, for a picture whose size was unknown. */
const loaded = ref<PixelSize | null>(null)
/** The card's copy has loaded: the checkerboard shows only under a picture, never in an empty box. */
const ready = ref(false)
const known = computed(() => props.size ?? loaded.value)
const page = usePreviewPage()
const box = computed(() => known.value ? previewBox(known.value, page.value.pixelRatio, page.value.size) : null)
/** The picture's size on the side the card went, scaled down when neither side had room for `box`; null until placed. */
const placed = ref<PixelSize | null>(null)
const shown = computed(() => placed.value ?? box.value)
/** The most a picture whose size is still unknown takes, so it never covers the page as it loads. */
const limit = computed(() => previewLimit(page.value.size))
const pictureStyle = computed(() => shown.value
  ? { width: `${shown.value.width}px`, height: `${shown.value.height}px` }
  : { maxWidth: `${limit.value.width}px`, maxHeight: `${limit.value.height}px` })
watch(() => props.src, () => {
  loaded.value = null
})

/**
 * Picks the card's side and its picture's size there (`previewPlacement`)
 * each time the card is positioned, so it follows the picture's moving and
 * the window's resizing. A picture whose size is unknown yet is placed as it
 * shows, unscaled. The card is kept within the page across by the tooltip,
 * and never moved up or down over the picture, which this sizes it to stay off.
 */
function placeCard({ reference, floating, placement }: { reference: HTMLElement; floating: HTMLElement; placement: Placement }): Placement {
  const image = picture.value
  if (!image)
    return placement
  const own = box.value ?? { width: image.offsetWidth, height: image.offsetHeight }
  const rect = reference.getBoundingClientRect()
  const next = previewPlacement(own, floating.offsetHeight - image.offsetHeight, {
    above: rect.top,
    below: page.value.size.height - rect.bottom,
  })
  const size = box.value ? next.picture : null
  if (size?.width !== placed.value?.width || size?.height !== placed.value?.height)
    placed.value = size
  return next.side
}

/** Each showing places the card anew for where the picture is now. */
function onOpen(): void {
  ready.value = false
  placed.value = null
}

// A phone has no hover, and shows no card (`file-previews.md`).
const phone = usePhone()

function onLoaded(event: Event): void {
  if (event.target instanceof HTMLImageElement || event.target instanceof HTMLVideoElement) {
    loaded.value = loadedSize(event.target)
    ready.value = true
  }
}
</script>

<template>
  <!-- A glance at the picture, which its trigger names already: the card takes no pointer and is not read twice. -->
  <Tooltip
    v-bind="$attrs"
    :tag="tag"
    :trigger="trigger"
    card
    placement="top"
    :open-delay-ms="OPEN_DELAY_MS"
    :warm-ms="WARM_MS"
    :disabled="phone"
    :place-by="placeCard"
    @open="onOpen"
  >
    <slot />
    <template #overlay>
      <div
        class="text-fg"
        :style="{
          padding: `${CARD_PAD}px`,
          width: shown ? `${Math.max(shown.width, CARD_MIN_WIDTH) + 2 * CARD_PAD}px` : undefined,
          maxWidth: shown ? undefined : `${limit.width + 2 * CARD_PAD}px`,
        }"
        aria-hidden="true"
        data-picture-preview
      >
        <div class="flex justify-center">
          <img
            v-if="kind === 'image'"
            ref="picture"
            :src="src"
            alt=""
            class="block rounded-md object-contain"
            :class="ready ? 'checkerboard' : ''"
            :style="pictureStyle"
            @load="onLoaded"
          >
          <video
            v-else
            ref="picture"
            :src="src"
            muted
            playsinline
            preload="metadata"
            class="block rounded-md bg-black object-contain"
            :style="pictureStyle"
            @loadedmetadata="onLoaded"
          />
        </div>
        <div class="flex min-w-0 items-baseline gap-2 px-1 pt-1.5 text-detail leading-4">
          <span class="min-w-0 truncate">{{ caption }}</span>
          <span v-if="known" class="shrink-0 tabular-nums text-fg-faint">{{ known.width }} × {{ known.height }}</span>
        </div>
      </div>
    </template>
  </Tooltip>
</template>
