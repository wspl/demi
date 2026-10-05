<script setup lang="ts">
import { computed, onBeforeUnmount, ref, useSlots, watch } from 'vue'
import { CircleX } from '@lucide/vue'
import ChromeRoll from '@demicodes/web-ui/ui/ChromeRoll.vue'
import Fold from '@demicodes/web-ui/ui/Fold.vue'
import FoldChevron from '@demicodes/web-ui/ui/FoldChevron.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import { useFollowEnd } from '../../composables/useFollowEnd'

const ACTIVE_OUTPUT_CLOSE_DELAY_MS = 1000

const props = defineProps<{
  label?: string
  detail?: string
  loading?: boolean
  /** The failure text, shown under the body. A failure as it happens opens the block and keeps it open; a block mounted over a past failure starts closed like any other. Pair with `tone="danger"`. */
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
}>()

const slots = useSlots()
const isOpen = defineModel<boolean>('open', { default: false })
const hasBodySlot = () => !!slots['body']
const hasPinnedSlot = () => !!slots['pinned']
// Read at render time: a conditional icon slot changes between renders, and slots are not reactive.
const showIcon = () => !!slots['icon'] || props.tone === 'danger'
const isExpandable = computed(() => props.expandable || hasPinnedSlot() || hasBodySlot() || !!props.errorText)
const bodyScroll = ref<HTMLElement>()
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

// A failed call keeps its error text on screen: the settle timer never closes over it.
// Only changes while mounted open a block; a transcript opened later shows every block folded.
function closeAfterActiveOutputSettles() {
  if (!isOpen.value)
    return
  clearCloseTimer()
  closeTimer = setTimeout(() => {
    closeTimer = undefined
    if (!props.openWhile && !props.errorText)
      isOpen.value = false
  }, ACTIVE_OUTPUT_CLOSE_DELAY_MS)
}

watch(
  [() => props.errorText, () => props.openWhile, isExpandable],
  ([errorText, openWhile, expandable]) => {
    clearCloseTimer()
    if (errorText) {
      isOpen.value = true
      return
    }
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
        </div>
      </ChromeRoll>
      <span class="-ml-1 shrink-0 text-xs">
        <FoldChevron
          v-if="isExpandable"
          :open="isOpen"
          :class="tone === 'danger' ? 'text-on-danger-muted group-hover:text-on-danger' : 'text-fg-faint group-hover:text-fg-muted'"
        />
      </span>
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
        <div class="flex max-h-80 min-w-0 flex-1 flex-col py-0.5">
          <div
            class="flex min-h-0 flex-col"
            :class="framed ? 'mx-3 my-1 rounded-md border border-line-subtle bg-surface-base py-2' : ''"
          >
            <div v-if="hasPinnedSlot()" class="max-h-40 shrink-0 overflow-y-auto">
              <slot name="pinned" />
            </div>
            <div
              ref="bodyScroll"
              class="min-h-0 overflow-y-auto"
              :class="hasPinnedSlot() && (hasBodySlot() || errorText) ? 'mt-1' : ''"
            >
              <div ref="bodyContent">
                <slot v-if="hasBodySlot()" name="body" />
                <pre
                  v-if="errorText"
                  class="whitespace-pre-wrap px-3 py-1.5 font-mono text-xs text-on-danger-muted"
                >{{ errorText }}</pre>
              </div>
            </div>
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
