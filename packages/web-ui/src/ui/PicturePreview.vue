<script lang="ts">
/**
 * The one card that shows at a time, and when the last one closed: moving
 * from one picture to the next shows the next one's card at once, as
 * tooltips after the first do.
 */
let openCard: { close(): void } | null = null
let closedAt = Number.NEGATIVE_INFINITY

/** How long the pointer rests on a picture before its card shows. */
const OPEN_DELAY_MS = 300
/** How soon after a card closed the next picture's shows without the delay. */
const WARM_MS = 300
/** The most a card's picture takes; a smaller one keeps its size. */
const PREVIEW_FRAME = { width: 480, height: 360 }
/** The narrowest a card is, so a small picture's line still reads. */
const CARD_MIN_WIDTH = 240
</script>

<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { autoUpdate, flip, getOverflowAncestors, offset, shift, useFloating } from '@floating-ui/vue'
import { fitScale } from '../files/image-zoom'
import { loadedSize, type PixelSize } from '../files/thumbnail'
import { useLayerElevation } from '../overlay/layerElevation'

defineOptions({ inheritAttrs: false })

/**
 * A picture's preview on hover (`file-previews.md` § A picture's preview
 * on hover), for every picture the page shows smaller than the card: a
 * row's pictures, a message run's thumbnails, a message's image capsule.
 * Resting the pointer on the picture for 300 ms shows a card above it, or
 * below it when the page has no room above, kept within the page: the
 * picture fitted within 480 × 360 and never enlarged, over a checkerboard
 * where it is transparent, a video's first frame, and under it one line,
 * what the picture belongs to, cut at its end, then its pixel size. Its box
 * comes from `size`, known before the bytes arrive, so it never grows as
 * they load. The card takes no pointer: leaving the picture closes it at
 * once, and a press on the picture, which opens the viewer, closes it. A
 * touch shows none.
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

const wrapper = ref<HTMLElement | null>(null)
const anchor = computed(() => props.trigger ?? wrapper.value)
const card = ref<HTMLElement | null>(null)
const open = ref(false)
const elevation = useLayerElevation()
/** The picture's size once the card's copy loaded, for a picture whose size was unknown. */
const loaded = ref<PixelSize | null>(null)
/** The card's copy has loaded: the checkerboard shows only under a picture, never in an empty box. */
const ready = ref(false)
const known = computed(() => props.size ?? loaded.value)
const box = computed(() => {
  const size = known.value
  if (!size)
    return null
  const scale = fitScale(size, PREVIEW_FRAME)
  return { width: Math.round(size.width * scale), height: Math.round(size.height * scale) }
})
watch(() => props.src, () => {
  loaded.value = null
})

const { floatingStyles } = useFloating(anchor, card, {
  placement: 'top',
  strategy: 'fixed',
  middleware: [offset(8), flip({ fallbackPlacements: ['bottom'] }), shift({ padding: 8 })],
  whileElementsMounted: autoUpdate,
  transform: false,
})

let openTimer: ReturnType<typeof setTimeout> | undefined
/** The picture was pressed: no card until the pointer leaves it. */
let pressed = false
let scrollTargets: EventTarget[] = []
const self = { close }

function clearOpenTimer(): void {
  if (openTimer !== undefined) {
    clearTimeout(openTimer)
    openTimer = undefined
  }
}

function show(): void {
  clearOpenTimer()
  if (open.value)
    return
  if (openCard && openCard !== self)
    openCard.close()
  openCard = self
  ready.value = false
  open.value = true
  // The picture moving away under a resting pointer, as the transcript scrolls, leaves it: the card goes.
  scrollTargets = anchor.value ? getOverflowAncestors(anchor.value) : []
  for (const target of scrollTargets)
    target.addEventListener('scroll', close, { passive: true })
}

function close(): void {
  clearOpenTimer()
  if (!open.value)
    return
  open.value = false
  for (const target of scrollTargets)
    target.removeEventListener('scroll', close)
  scrollTargets = []
  if (openCard === self) {
    openCard = null
    closedAt = performance.now()
  }
}

function onPointerenter(event: PointerEvent): void {
  if (pressed || event.pointerType === 'touch')
    return
  clearOpenTimer()
  if (openCard !== null || performance.now() - closedAt < WARM_MS) {
    show()
    return
  }
  openTimer = setTimeout(show, OPEN_DELAY_MS)
}

function onPointerleave(): void {
  pressed = false
  close()
}

function onPointerdown(): void {
  pressed = true
  close()
}

function onLoaded(event: Event): void {
  if (event.target instanceof HTMLImageElement || event.target instanceof HTMLVideoElement) {
    loaded.value = loadedSize(event.target)
    ready.value = true
  }
}

/** Gives a `trigger` element the listeners the slot's wrapper has in the template, or takes them away. */
function listen(element: HTMLElement, on: boolean): void {
  if (on) {
    element.addEventListener('pointerenter', onPointerenter)
    element.addEventListener('pointerleave', onPointerleave)
    element.addEventListener('pointerdown', onPointerdown, { capture: true })
    return
  }
  element.removeEventListener('pointerenter', onPointerenter)
  element.removeEventListener('pointerleave', onPointerleave)
  element.removeEventListener('pointerdown', onPointerdown, { capture: true })
}

watch(() => props.trigger, (element, before) => {
  if (before)
    listen(before, false)
  if (element)
    listen(element, true)
}, { immediate: true })

onBeforeUnmount(() => {
  close()
  if (props.trigger)
    listen(props.trigger, false)
})
</script>

<template>
  <component
    :is="tag ?? 'span'"
    v-if="!trigger"
    ref="wrapper"
    v-bind="$attrs"
    @pointerenter="onPointerenter"
    @pointerleave="onPointerleave"
    @pointerdown.capture="onPointerdown"
  >
    <slot />
  </component>
  <Teleport to="body">
    <Transition
      enter-active-class="transition-[opacity,scale] duration-120 ease-out"
      leave-active-class="transition-[opacity,scale] duration-100 ease-out"
      enter-from-class="opacity-0 scale-95"
      leave-to-class="opacity-0 scale-95"
    >
      <!-- A glance at the picture, which its trigger names already: no pointer, and not read twice. -->
      <div
        v-if="open"
        ref="card"
        class="overlay-shell pointer-events-none z-[60] select-none rounded-lg p-1.5 text-fg"
        :style="{
          ...floatingStyles,
          '--elevation': elevation,
          width: box ? `${Math.max(box.width, CARD_MIN_WIDTH) + 12}px` : undefined,
          maxWidth: box ? undefined : `${PREVIEW_FRAME.width + 12}px`,
        }"
        aria-hidden="true"
        data-picture-preview
      >
        <div class="flex justify-center">
          <img
            v-if="kind === 'image'"
            :src="src"
            alt=""
            class="block rounded-md object-contain"
            :class="ready ? 'checkerboard' : ''"
            :style="box
              ? { width: `${box.width}px`, height: `${box.height}px` }
              : { maxWidth: `${PREVIEW_FRAME.width}px`, maxHeight: `${PREVIEW_FRAME.height}px` }"
            @load="onLoaded"
          >
          <video
            v-else
            :src="src"
            muted
            playsinline
            preload="metadata"
            class="block rounded-md bg-black object-contain"
            :style="box
              ? { width: `${box.width}px`, height: `${box.height}px` }
              : { maxWidth: `${PREVIEW_FRAME.width}px`, maxHeight: `${PREVIEW_FRAME.height}px` }"
            @loadedmetadata="onLoaded"
          />
        </div>
        <div class="flex min-w-0 items-baseline gap-2 px-1 pt-1.5 text-xs leading-4">
          <span class="min-w-0 truncate">{{ caption }}</span>
          <span v-if="known" class="shrink-0 tabular-nums text-fg-faint">{{ known.width }} × {{ known.height }}</span>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>
