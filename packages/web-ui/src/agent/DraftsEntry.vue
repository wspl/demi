<script setup lang="ts">
import { computed } from 'vue'
import { NotebookPen } from '@lucide/vue'
import DropdownTrigger from '../ui/DropdownTrigger.vue'
import { ICON_PX } from '../ui/icon-metrics'
import Tooltip from '../ui/Tooltip.vue'
import { draftsLabel } from './drafts'

/**
 * The drafts' entry at the left of the row under the composer
 * (`product.md` § Drafts): a quiet chip with the count, like the model's at
 * the other end. Pressing it opens the list under the composer, or closes
 * it. The caller shows it only while there is a draft. Where the row has no
 * room (`compact`), it keeps its mark and the count alone, and its tip reads
 * the whole label.
 */
const props = defineProps<{
  count: number
  open: boolean
  compact?: boolean
}>()

const emit = defineEmits<{ toggle: [] }>()
const label = computed(() => draftsLabel(props.count))
</script>

<template>
  <Tooltip :content="label" :disabled="!compact">
    <DropdownTrigger
      variant="ghost"
      :is-open="open"
      :aria-label="label"
      aria-haspopup="true"
      :aria-expanded="open"
      @click="emit('toggle')"
    >
      <NotebookPen :size="ICON_PX.in28" class="mr-1 shrink-0" />
      <span class="whitespace-nowrap">{{ compact ? count : label }}</span>
    </DropdownTrigger>
  </Tooltip>
</template>
