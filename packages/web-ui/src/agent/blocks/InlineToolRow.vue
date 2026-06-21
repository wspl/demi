<script setup lang="ts">
import { computed, ref, useSlots, watch } from 'vue'
import { RightSmallLine } from '@mingcute/vue/right-small'
import ToolStatusBadge from './ToolStatusBadge.vue'
import IndeterminateSpinner from '@demi/web-ui/ui/IndeterminateSpinner.vue'

const props = defineProps<{
  label: string
  detail?: string
  suffix?: string
  trailing?: string
  loading?: boolean
  error?: boolean
  errorText?: string
}>()

const slots = useSlots()
const isOpen = defineModel<boolean>('open', { default: false })
const hasBodySlot = () => !!slots['body']
const isExpandable = computed(() => hasBodySlot() || !!props.errorText)
const isHovered = ref(false)

watch(() => props.errorText, (text) => {
  if (text) isOpen.value = true
})
</script>

<template>
  <div class="overflow-hidden">
    <div
      class="flex items-center gap-2 py-1 text-[13px] text-fg-muted"
      :class="isExpandable ? 'cursor-pointer' : ''"
      @click="isExpandable && (isOpen = !isOpen)"
      @mouseenter="isHovered = true"
      @mouseleave="isHovered = false"
    >
      <div class="flex size-4 shrink-0 items-center justify-center">
        <slot name="icon" />
      </div>
      <div class="flex min-w-0 flex-1 items-center gap-2 overflow-hidden">
        <span class="shrink-0">{{ label }}</span>
        <slot v-if="slots['default']" />
        <span v-else-if="detail" class="truncate font-mono text-fg-body">{{ detail }}</span>
        <span v-if="suffix" class="shrink-0 text-fg-subtle">{{ suffix }}</span>
      </div>
      <span class="shrink-0 text-xs">
        <template v-if="isExpandable && (isHovered || isOpen)">
          <RightSmallLine :size="18" class="text-fg-faint" :class="isOpen ? 'rotate-90' : ''" />
        </template>
        <template v-else>
          <ToolStatusBadge v-if="error" status="error" />
          <IndeterminateSpinner v-else-if="loading" :size="12" :stroke-width="2" arc-class="text-on-warning" track-class="text-tint-warning" />
          <span v-else-if="trailing" class="text-fg-subtle">{{ trailing }}</span>
        </template>
      </span>
    </div>
    <div v-if="isExpandable" class="collapsible-body" :class="isOpen ? 'is-open' : ''">
      <div class="overflow-hidden">
        <pre v-if="errorText" class="whitespace-pre-wrap px-6 py-2 font-mono text-xs text-on-danger-muted">{{ errorText }}</pre>
        <slot v-if="hasBodySlot()" name="body" />
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
