<script setup lang="ts">
import { computed, useSlots } from 'vue'
import ToolStatusBadge from './ToolStatusBadge.vue'
import IndeterminateSpinner from '@demi/web-ui/ui/IndeterminateSpinner.vue'

const props = withDefaults(defineProps<{
  error?: boolean
  loading?: boolean
  showBody?: boolean
  headerClickable?: boolean
  collapsedHeight?: string
  errorText?: string
}>(), {
  showBody: true,
})

const emit = defineEmits<{
  'header-click': []
}>()

const slots = useSlots()
const isBodyVisible = computed(() => props.showBody || !!props.errorText)
const isBodyTopVisible = computed(() => !!slots['body-top'])

function handleHeaderClick() {
  if (!props.headerClickable) return
  emit('header-click')
}
</script>

<template>
  <div
    class="group overflow-hidden rounded-lg border"
    :class="[
      error ? 'border-on-danger-muted' : 'border-line',
      headerClickable ? 'cursor-pointer' : '',
    ]"
    @click="handleHeaderClick"
  >
    <div
      class="flex items-center gap-2 bg-overlay/2 px-2 py-2 transition-colors"
      :class="headerClickable ? 'group-hover:bg-hover' : ''"
    >
      <div class="flex size-5 shrink-0 items-center justify-center text-fg-muted">
        <slot name="icon" />
      </div>

      <div class="flex min-w-0 flex-1 items-center gap-2 overflow-hidden">
        <slot name="header" />
      </div>

      <div class="flex h-5 shrink-0 items-center gap-1.5 text-xs">
        <template v-if="loading">
          <IndeterminateSpinner :size="12" :stroke-width="2" />
        </template>
        <ToolStatusBadge v-else-if="error" status="error" />
        <slot v-else name="stats" />
      </div>
    </div>

    <div v-if="isBodyVisible" class="border-t border-line bg-surface">
      <div v-if="isBodyTopVisible">
        <slot name="body-top" />
      </div>

      <pre v-if="errorText" class="whitespace-pre-wrap px-3 py-2 font-mono text-xs text-on-danger-muted">{{ errorText }}</pre>
      <div v-if="showBody" class="relative">
        <div
          class="flex flex-col-reverse overflow-hidden"
          :style="{ maxHeight: collapsedHeight }"
        >
          <slot name="body" />
        </div>
      </div>
    </div>
  </div>
</template>
