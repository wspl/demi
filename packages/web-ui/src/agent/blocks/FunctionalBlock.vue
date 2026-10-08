<script setup lang="ts">
import { computed, onBeforeUnmount, ref, useSlots, watch } from 'vue'
import { CircleX } from '@lucide/vue'
import ChromeRoll from '@demicodes/web-ui/ui/ChromeRoll.vue'
import Fold from '@demicodes/web-ui/ui/Fold.vue'
import FoldChevron from '@demicodes/web-ui/ui/FoldChevron.vue'
import ScrollArea from '@demicodes/web-ui/ui/ScrollArea.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import { useFollowEnd } from '../../composables/useFollowEnd'

const ACTIVE_OUTPUT_CLOSE_DELAY_MS = 1000

const props = defineProps<{
  label?: string
  detail?: string
  loading?: boolean
  /** The failure text, shown under the body once the user opens the block; a failure never opens it. Pair with `tone="danger"`. */
  errorText?: string
  tone?: 'danger'
  /** Keep the scrolling part at its latest line while content streams in (live thinking, a running command's output), until the reader scrolls up from it. */
  stickBottom?: boolean
  /** Draw the body as a box, as a shell call's command and output. */
  framed?: boolean
  /** Keep the block open while active output is being produced. */
  openWhile?: boolean
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
  /** Show the chevron of the row this face hands over to, without being a control itself (the tail row's face). */
  chevron?: boolean
}>()

const slots = useSlots()
const isOpen = defineModel<boolean>('open', { default: false })
const hasBodySlot = () => !!slots['body']
const hasPinnedSlot = () => !!slots['pinned']
// Read at render time: a conditional icon slot changes between renders, and slots are not reactive.
const showIcon = () => !!slots['icon'] || props.tone === 'danger'
const isExpandable = computed(() => props.expandable || hasPinnedSlot() || hasBodySlot() || !!props.errorText)
const bodyArea = ref<InstanceType<typeof ScrollArea>>()
const bodyScroll = computed(() => bodyArea.value?.el)
const bodyContent = ref<HTMLElement>()
let closeTimer: ReturnType<typeof setTimeout> | undefined

function toggleOpen(): void {
  if (isExpandable.value) {
    isOpen.value = !isOpen.value
  }
}

function clearCloseTimer() {
  if (!closeTimer)
    return
  clearTimeout(closeTimer)
  closeTimer = undefined
}

// Only changes while mounted open a block; a transcript opened later shows every block folded.
function closeAfterActiveOutputSettles() {
  if (!isOpen.value)
    return
  clearCloseTimer()
  closeTimer = setTimeout(() => {
    closeTimer = undefined
    if (!props.openWhile)
      isOpen.value = false
  }, ACTIVE_OUTPUT_CLOSE_DELAY_MS)
}

watch(
  [() => props.openWhile, isExpandable],
  ([openWhile, expandable]) => {
    clearCloseTimer()
    if (openWhile === undefined)
      return
    if (openWhile && expandable) {
      isOpen.value = true
      return
    }
    closeAfterActiveOutputSettles()
  },
)

onBeforeUnmount(clearCloseTimer)

useFollowEnd(bodyScroll, bodyContent, () => !!props.stickBottom)
</script>

<template>
  <div class="overflow-hidden">
    <div
      class="flex h-7 cursor-default select-none items-center gap-2 text-chrome transition-colors duration-200 ease-out"
      :class="tone === 'danger'
        ? isExpandable ? 'group text-on-danger hover:text-on-danger' : 'text-on-danger'
        : isExpandable ? 'group text-fg-muted hover:text-fg-body' : 'text-fg-muted'"
      :role="isExpandable ? 'button' : undefined"
      :tabindex="isExpandable ? 0 : undefined"
      :aria-expanded="isExpandable ? isOpen : undefined"
      @click="toggleOpen"
      @keydown.enter.self.prevent="toggleOpen"
      @keydown.space.self.prevent="toggleOpen"
    >
      <ChromeRoll
        class="min-w-0"
        :face-key="rollKey ?? 'static'"
        :icon-key="iconKey ?? 'icon'"
        :cut-key="cutKey"
      >
        <template v-if="showIcon()" #icon>
          <div
            class="functional-block-icon flex shrink-0 items-center justify-center"
            :style="{ width: `${ICON_PX.in28}px`, height: `${ICON_PX.in28}px` }"
          >
            <slot name="icon">
              <CircleX :size="ICON_PX.in28" />
            </slot>
          </div>
        </template>
        <div class="flex h-7 min-w-0 items-center gap-2 overflow-hidden">
          <span
            v-if="label"
            class="shrink-0"
            :class="loading ? 'thinking-shimmer' : ''"
          >{{ label }}</span>
          <slot v-if="slots['default']" :loading="loading" />
          <span
            v-else-if="detail"
            class="min-w-0 truncate font-mono text-fg-body group-hover:text-fg-emphasis"
            :class="loading ? 'thinking-shimmer' : ''"
          >{{ detail }}</span>
          <!-- The chevron is part of the face: it follows the label's end, so it rolls with it rather than jumping to the new label's width. -->
          <span v-if="isExpandable || chevron" class="-ml-1 shrink-0 text-xs">
            <FoldChevron
              :open="isOpen"
              :class="tone === 'danger' ? 'text-on-danger-muted group-hover:text-on-danger' : 'text-fg-faint group-hover:text-fg-muted'"
            />
          </span>
        </div>
      </ChromeRoll>
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
                  class="whitespace-pre-wrap px-3 py-1.5 font-mono text-xs text-on-danger-muted"
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
