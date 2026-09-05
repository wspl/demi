<script setup lang="ts">
import { computed, onBeforeUnmount, onUpdated, ref, useSlots, watch } from 'vue'
import { ChevronRight, CircleX } from '@lucide/vue'
import ChromeRoll from '@demicodes/web-ui/ui/ChromeRoll.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'

const ACTIVE_OUTPUT_CLOSE_DELAY_MS = 1000

const props = defineProps<{
  label?: string
  detail?: string
  suffix?: string
  trailing?: string
  loading?: boolean
  /** The failure text, shown under the body; keeps the block open. Pair with `tone="danger"`. */
  errorText?: string
  tone?: 'danger'
  /** Keep the body scrolled to the latest line while content streams in (e.g. live thinking). */
  stickBottom?: boolean
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
const showIcon = computed(() => !!slots['icon'] || props.tone === 'danger')
const isExpandable = computed(() => props.expandable || hasBodySlot() || !!props.errorText)
const bodyScroll = ref<HTMLElement>()
let closeTimer: ReturnType<typeof setTimeout> | undefined

function clearCloseTimer() {
  if (!closeTimer) return
  clearTimeout(closeTimer)
  closeTimer = undefined
}

// A failed call keeps its error text on screen: the settle timer never closes over it.
function closeAfterActiveOutputSettles() {
  if (!isOpen.value) return
  clearCloseTimer()
  closeTimer = setTimeout(() => {
    closeTimer = undefined
    if (!props.openWhile && !props.errorText) isOpen.value = false
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
    if (openWhile === undefined) return
    if (openWhile && expandable) {
      isOpen.value = true
      return
    }
    closeAfterActiveOutputSettles()
  },
  { immediate: true },
)

onBeforeUnmount(clearCloseTimer)

onUpdated(() => {
  if (props.stickBottom && isOpen.value && bodyScroll.value) {
    bodyScroll.value.scrollTop = bodyScroll.value.scrollHeight
  }
})
</script>

<template>
  <div class="overflow-hidden">
    <div
      class="flex h-7 cursor-default select-none items-center gap-2 text-chrome transition-colors duration-200 ease-out"
      :class="tone === 'danger'
        ? isExpandable ? 'group text-on-danger hover:text-on-danger' : 'text-on-danger'
        : isExpandable ? 'group text-fg-muted hover:text-fg-body' : 'text-fg-muted'"
      @click="isExpandable && (isOpen = !isOpen)"
    >
      <ChromeRoll class="min-w-0" :face-key="rollKey ?? 'static'" :icon-key="iconKey ?? 'icon'">
        <template v-if="showIcon" #icon>
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
          <span v-if="label" class="shrink-0" :class="loading ? 'thinking-shimmer' : ''">{{ label }}</span>
          <slot v-if="slots['default']" :loading="loading" />
          <span v-else-if="detail" class="min-w-0 truncate font-mono text-fg-body group-hover:text-fg-emphasis" :class="loading ? 'thinking-shimmer' : ''">{{ detail }}</span>
          <span v-if="suffix" class="shrink-0 text-fg-subtle group-hover:text-fg-muted" :class="loading ? 'thinking-shimmer' : ''">{{ suffix }}</span>
        </div>
      </ChromeRoll>
      <span class="-ml-1 shrink-0 text-xs">
        <ChevronRight
          v-if="isExpandable"
          :size="ICON_PX.in28"
          class="transition-[color,transform] duration-200"
          :class="[
            isOpen ? 'rotate-90' : '',
            tone === 'danger' ? 'text-on-danger-muted group-hover:text-on-danger' : 'text-fg-faint group-hover:text-fg-muted',
          ]"
        />
        <span v-else-if="!loading && trailing" class="text-fg-subtle group-hover:text-fg-muted">{{ trailing }}</span>
      </span>
      <div class="flex-1"></div>
    </div>
    <div v-if="isExpandable" class="functional-block-body" :class="isOpen ? 'is-open' : ''">
      <div class="overflow-hidden">
        <div class="mb-1 flex overflow-hidden">
          <div
            v-if="showIcon"
            class="functional-block-rail shrink-0"
            :style="{ width: `${ICON_PX.in28}px` }"
          />
          <div ref="bodyScroll" class="min-w-0 max-h-80 flex-1 overflow-y-auto py-0.5">
            <slot v-if="hasBodySlot()" name="body" />
            <pre v-if="errorText" class="whitespace-pre-wrap px-3 py-1.5 font-mono text-xs text-on-danger-muted">{{ errorText }}</pre>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.functional-block-body {
  display: grid;
  grid-template-rows: 0fr;
  transition: grid-template-rows 0.2s ease;
}

.functional-block-body.is-open {
  grid-template-rows: 1fr;
}

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
