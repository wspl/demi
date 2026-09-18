<script setup lang="ts">
import { computed } from 'vue'
import { Globe, X } from '@lucide/vue'
import IconButton from '../ui/IconButton.vue'
import MenuItem from '../ui/MenuItem.vue'
import { formatTimeRemaining, useTimeRemaining } from '../composables/useRelativeTime'
import type { ExposeMenuEntry } from './types'

/**
 * One expose row of the session tools menu: the address, the host in
 * parentheses, and a countdown on the shared clock. Selecting the row opens
 * the URL; the suffix control removes the expose without leaving the menu.
 */
const props = defineProps<{
  expose: ExposeMenuEntry
  /** A remove request is in flight. */
  pending?: boolean
}>()
const emit = defineEmits<{
  open: []
  remove: []
}>()
const remaining = useTimeRemaining(() => props.expose.expiresAt)
const countdown = computed(() => formatTimeRemaining(remaining.value))
</script>

<template>
  <MenuItem
    :icon="Globe"
    :label="expose.address"
    :note="expose.hostName"
    :value="countdown"
    :disabled="pending"
    :title="expose.url"
    @select="emit('open')"
  >
    <template #suffix>
      <IconButton
        :icon="X"
        size="xs"
        variant="ghost"
        aria-label="Remove expose"
        :loading="pending"
        @click.stop="emit('remove')"
      />
    </template>
  </MenuItem>
</template>
