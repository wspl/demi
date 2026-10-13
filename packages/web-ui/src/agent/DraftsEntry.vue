<script setup lang="ts">
import { computed } from 'vue'
import { NotebookPen } from '@lucide/vue'
import DropdownTrigger from '../ui/DropdownTrigger.vue'
import { ICON_PX } from '../ui/icon-metrics'
import { draftsLabel } from './drafts'

/**
 * The drafts' entry at the left of the row under the composer
 * (`product.md` § Drafts): a quiet chip with the count, like the model's at
 * the other end. Pressing it opens the list under the composer, or closes
 * it. The caller shows it only while there is a draft.
 */
const props = defineProps<{
  count: number
  open: boolean
}>()

const emit = defineEmits<{ toggle: [] }>()
const label = computed(() => draftsLabel(props.count))
</script>

<template>
  <DropdownTrigger
    variant="ghost"
    class="max-w-full"
    :is-open="open"
    :aria-label="label"
    aria-haspopup="true"
    :aria-expanded="open"
    @click="emit('toggle')"
  >
    <NotebookPen :size="ICON_PX.in28" class="mr-1 shrink-0" />
    <span class="min-w-0 truncate">{{ label }}</span>
  </DropdownTrigger>
</template>
