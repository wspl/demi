<script setup lang="ts">
import { computed, onUpdated, ref, useSlots, watch } from 'vue'
import { RightLine } from '@mingcute/vue/right'
import ToolStatusBadge from './ToolStatusBadge.vue'
import IndeterminateSpinner from '@demi/web-ui/ui/IndeterminateSpinner.vue'

const props = defineProps<{
  label?: string
  detail?: string
  suffix?: string
  trailing?: string
  loading?: boolean
  error?: boolean
  errorText?: string
  /** Keep the body scrolled to the latest line while content streams in (e.g. live thinking). */
  stickBottom?: boolean
  /** Force expandability instead of inferring it from the body slot (slot presence isn't reactive). */
  expandable?: boolean
}>()

const slots = useSlots()
const isOpen = defineModel<boolean>('open', { default: false })
const hasBodySlot = () => !!slots['body']
const isExpandable = computed(() => props.expandable || hasBodySlot() || !!props.errorText)
const bodyScroll = ref<HTMLElement>()

watch(() => props.errorText, (text) => {
  if (text) isOpen.value = true
})

onUpdated(() => {
  if (props.stickBottom && isOpen.value && bodyScroll.value) {
    bodyScroll.value.scrollTop = bodyScroll.value.scrollHeight
  }
})
</script>

<template>
  <div class="overflow-hidden">
    <div
      class="flex select-none items-center gap-2 py-1 text-[13px] text-fg-muted"
      @click="isExpandable && (isOpen = !isOpen)"
    >
      <div class="flex size-4 shrink-0 items-center justify-center">
        <slot name="icon" />
      </div>
      <div class="flex min-w-0 items-center gap-2 overflow-hidden">
        <span v-if="label" class="shrink-0">{{ label }}</span>
        <slot v-if="slots['default']" />
        <span v-else-if="detail" class="min-w-0 truncate font-mono text-fg-body">{{ detail }}</span>
        <span v-if="suffix" class="shrink-0 text-fg-subtle">{{ suffix }}</span>
      </div>
      <span class="-ml-1 shrink-0 text-xs">
        <ToolStatusBadge v-if="error" status="error" />
        <IndeterminateSpinner v-else-if="loading" :size="12" :stroke-width="2" arc-class="text-on-warning" track-class="text-tint-warning" />
        <RightLine v-else-if="isExpandable" :size="14" class="text-fg-faint transition-transform duration-200" :class="isOpen ? 'rotate-90' : ''" />
        <span v-else-if="trailing" class="text-fg-subtle">{{ trailing }}</span>
      </span>
      <div class="flex-1"></div>
    </div>
    <div v-if="isExpandable" class="collapsible-body" :class="isOpen ? 'is-open' : ''">
      <div class="overflow-hidden">
        <div class="mb-1 ml-6 overflow-hidden rounded-md bg-overlay/6">
          <div ref="bodyScroll" class="max-h-80 overflow-y-auto py-0.5">
            <pre v-if="errorText" class="whitespace-pre-wrap px-3 py-1.5 font-mono text-xs text-on-danger-muted">{{ errorText }}</pre>
            <slot v-if="hasBodySlot()" name="body" />
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.collapsible-body {
  display: grid;
  grid-template-rows: 0fr;
  transition: grid-template-rows 0.2s ease;
}

.collapsible-body.is-open {
  grid-template-rows: 1fr;
}
</style>
