<script setup lang="ts">
import { computed, ref, useId, useSlots, watch } from 'vue'
import { useResizeObserver } from '@vueuse/core'
import ChromeRoll from '@demicodes/web-ui/ui/ChromeRoll.vue'
import Fold from '@demicodes/web-ui/ui/Fold.vue'
import FoldChevron from '@demicodes/web-ui/ui/FoldChevron.vue'
import ScrollArea from '@demicodes/web-ui/ui/ScrollArea.vue'
import Shimmer from '@demicodes/web-ui/ui/Shimmer.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import { textWidth } from '@demicodes/web-ui/ui/text-width'
import { useFollowEnd } from '../../composables/useFollowEnd'
import { useWholeBlock } from '../whole-blocks'
import RowPicture from '../RowPicture.vue'
import { fitRowPictures, rowPictureBox } from '../row-pictures'
import type { ToolPicture } from '../tool-media'

const props = defineProps<{
  loading?: boolean
  /** The failure text, shown under the body once the user opens the block; a failure never opens it. The face marks the failure. */
  errorText?: string
  /** Keep the scrolling part at its latest line while content streams in (live thinking, a running command's output), until the reader scrolls up from it. */
  stickBottom?: boolean
  /** Draw the body as a box, as a shell call's command and output. */
  framed?: boolean
  /** Force expandability instead of inferring it from the body slot (slot presence isn't reactive). */
  expandable?: boolean
  /** When this changes, the 28px label rolls. Omit to keep the face still. */
  rollKey?: string
  /** When this stays put, the icon does not roll with the label. */
  iconKey?: string
  /** When this changes, the face changes at once, without a roll (ChromeRoll's `cutKey`). */
  cutKey?: string
  /** The body is rows of their own, as a work group's steps: in flow, unbounded, never scrolled. */
  flow?: boolean
  /** The images and videos the row's step returned, small in its line after the face (`file-previews.md` § Media a tool returned). */
  pictures?: readonly ToolPicture[]
}>()

const slots = useSlots()
/** The face, which names the button that toggles the row. */
const faceId = useId()
const isOpen = defineModel<boolean>('open', { default: false })
const hasBodySlot = () => !!slots['body']
const hasPinnedSlot = () => !!slots['pinned']
// Read at render time: a conditional icon slot changes between renders, and slots are not reactive.
const showIcon = () => !!slots['icon']
// A row whose block the page holds light opens to read it whole, before its body is there.
const whole = useWholeBlock()
const isExpandable = computed(() => props.expandable || whole.light.value || hasPinnedSlot() || hasBodySlot() || !!props.errorText)
const isLoading = computed(() => props.loading || whole.loading.value)
const bodyArea = ref<InstanceType<typeof ScrollArea>>()
const bodyScroll = computed(() => bodyArea.value?.el)
const bodyContent = ref<HTMLElement>()

// Only the reader opens and folds a row (or its owner, through `open`): a
// row that opened and folded by itself as its content streamed and ended
// would move the transcript with nothing the reader did.
function toggle(): void {
  if (isExpandable.value) {
    isOpen.value = !isOpen.value
  }
}

// However the row opens, by its face or by its owner, a light block is read whole.
watch(isOpen, (open) => {
  if (open) {
    whole.open()
  }
})

useFollowEnd(bodyScroll, bodyContent, () => !!props.stickBottom)

// The face and the pictures share the line by one rule (`row-pictures.ts`):
// the title gives way first, down to half the row, then the pictures, into
// the count of the rest. The face's parts that never shrink, its tags and
// its chevron, stay whole throughout.
const faceRow = ref<HTMLElement>()
const roll = ref<InstanceType<typeof ChromeRoll>>()
const rowWidth = ref(0)
const faceWidth = ref(0)
const rowFont = ref('')
function measureFace(): void {
  const row = faceRow.value
  if (!row || !props.pictures?.length)
    return
  rowWidth.value = row.clientWidth
  faceWidth.value = roll.value?.naturalWidth() ?? 0
  rowFont.value = getComputedStyle(row).font
}
// Only a row with pictures watches its widths: a transcript holds hundreds of rows.
useResizeObserver(() => (props.pictures?.length ? faceRow.value : undefined), measureFace)
// The face's width changes with what it says, as a run's face rolls to its next step.
useResizeObserver(() => (props.pictures?.length ? roll.value?.root : undefined), measureFace)
watch(() => props.pictures?.length ?? 0, measureFace, { flush: 'post' })
const picturesFit = computed(() => {
  const pictures = props.pictures ?? []
  if (pictures.length === 0 || rowWidth.value === 0)
    return { shown: 0, more: 0 }
  const font = rowFont.value
  return fitRowPictures(rowWidth.value, faceWidth.value, pictures.map((picture) => rowPictureBox(picture).width), (count) => Math.ceil(textWidth(`+${count}`, font)))
})
const shownPictures = computed(() => (props.pictures ?? []).slice(0, picturesFit.value.shown))
</script>

<template>
  <div class="overflow-hidden">
    <!-- A click anywhere on the face toggles the row. The control that
      toggles it is the button laid under the face, so a control the face
      holds, such as a notice's title, is its sibling, never inside it: each
      has its own focus and name, and Enter or Space on the button clicks
      it, which reaches the face's click as a click on the face does. -->
    <div
      ref="faceRow"
      class="relative flex h-7 cursor-default select-none items-center gap-2 text-row transition-colors duration-200 ease-out"
      :class="isExpandable ? 'group text-fg-row hover:text-fg-muted' : 'text-fg-row'"
      @click="toggle"
    >
      <button
        v-if="isExpandable"
        type="button"
        class="absolute inset-0 cursor-default rounded-sm outline-offset-[-2px]"
        :aria-expanded="isOpen"
        :aria-labelledby="faceId"
      ></button>
      <ChromeRoll
        :id="faceId"
        ref="roll"
        class="relative min-w-0"
        :face-key="rollKey ?? 'static'"
        :icon-key="iconKey ?? 'icon'"
        :cut-key="cutKey"
      >
        <template v-if="showIcon()" #icon>
          <div
            class="functional-block-icon flex shrink-0 items-center justify-center"
            :style="{ width: `${ICON_PX.in28}px`, height: `${ICON_PX.in28}px` }"
          >
            <slot name="icon" />
          </div>
        </template>
        <!-- A row still working shimmers as one, whatever parts its face holds. -->
        <Shimmer :active="isLoading" class="h-7 items-center gap-2 overflow-hidden">
          <slot />
          <!-- The chevron is part of the face: it follows the label's end, so it rolls with it rather than jumping to the new label's width. -->
          <span v-if="isExpandable" class="-ml-1 shrink-0 text-xs">
            <FoldChevron
              :open="isOpen"
              class="text-fg-faint group-hover:text-fg-subtle"
            />
          </span>
        </Shimmer>
      </ChromeRoll>
      <!-- 8 px after the face, the face row's gap; one line, never wrapping, the rest counted. -->
      <div
        v-if="picturesFit.shown > 0 || picturesFit.more > 0"
        class="relative flex shrink-0 items-center gap-1"
        data-row-pictures
      >
        <RowPicture
          v-for="(picture, index) in shownPictures"
          :key="index"
          :picture="picture"
        />
        <span v-if="picturesFit.more > 0" class="tabular-nums text-fg-faint">+{{ picturesFit.more }}</span>
      </div>
      <div class="flex-1"></div>
    </div>
    <Fold v-if="isExpandable" :open="isOpen">
      <div class="mb-1 flex overflow-hidden">
        <div
          v-if="showIcon()"
          class="functional-block-rail shrink-0"
          :style="{ width: `${ICON_PX.in28}px` }"
        />
        <!-- The body is bounded and does not scroll: the pinned part (a call's
          input) stays at its top, under its own bound, and only the part
          below it scrolls. -->
        <!-- Rows of their own start where the label starts, so each row's icon stands under the title, clear of the rail. -->
        <div v-if="flow" class="min-w-0 flex-1 py-0.5 pl-2">
          <slot name="body" />
        </div>
        <div v-else class="flex max-h-80 min-w-0 flex-1 flex-col py-0.5">
          <div
            class="flex min-h-0 flex-col"
            :class="framed ? 'mx-3 my-1 rounded-md border border-line-subtle bg-surface-base py-2' : ''"
          >
            <ScrollArea v-if="hasPinnedSlot()" class="max-h-40 shrink-0">
              <slot name="pinned" />
            </ScrollArea>
            <ScrollArea
              ref="bodyArea"
              :class="hasPinnedSlot() && (hasBodySlot() || errorText) ? 'mt-1' : ''"
            >
              <div ref="bodyContent">
                <slot v-if="hasBodySlot()" name="body" />
                <pre
                  v-if="errorText"
                  class="whitespace-pre-wrap px-3 py-1.5 wrap-anywhere font-mono text-xs text-on-danger-muted"
                >{{ errorText }}</pre>
              </div>
            </ScrollArea>
          </div>
        </div>
      </div>
    </Fold>
  </div>
</template>

<style scoped>
.functional-block-icon :deep(svg) {
  width: 100%;
  height: 100%;
}

.functional-block-rail {
  position: relative;
}

.functional-block-rail::after {
  content: '';
  position: absolute;
  top: 0;
  bottom: 0;
  left: 50%;
  width: 1px;
  transform: translateX(-50%);
  background: var(--line-subtle);
}
</style>
