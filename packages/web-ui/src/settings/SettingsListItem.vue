<script setup lang="ts">
import { Trash2 } from '@lucide/vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'

/** One entry in a SettingsSplit list: its mark (the `leading` slot), the name, and a dot when its state matters. */
defineProps<{
  label: string
  selected?: boolean
  /** A dot on the mark's corner. Omit it when there is nothing to say. */
  badge?: 'success' | 'warning' | 'danger'
  muted?: boolean
  /** Shows a remove button on hover; the entry's page carries no such action. */
  removable?: boolean
  removing?: boolean
  removeDisabled?: boolean
}>()

const emit = defineEmits<{
  select: []
  remove: []
}>()
</script>

<template>
  <div
    role="button"
    :aria-pressed="selected"
    class="group flex h-8 cursor-default select-none items-center gap-2 rounded-md px-1 transition-colors duration-200 ease-out"
    :class="[
      selected ? 'bg-active' : 'hover:bg-hover',
      muted ? 'opacity-60' : '',
    ]"
    @click="emit('select')"
  >
    <span class="relative flex shrink-0 items-center">
      <slot name="leading" />
      <span
        v-if="badge"
        class="absolute -right-px -top-px size-1.5 rounded-full ring-1 ring-surface"
        :class="{
          'bg-on-success': badge === 'success',
          'bg-on-warning': badge === 'warning',
          'bg-on-danger': badge === 'danger',
        }"
      />
    </span>
    <span class="flex min-w-0 flex-1 flex-col leading-4">
      <span
        class="truncate text-chrome"
        :class="selected ? 'text-fg-emphasis' : 'text-fg'"
        >{{ label }}</span
      >
    </span>
    <span
      v-if="removable"
      :class="removing ? 'flex shrink-0' : 'hidden shrink-0 group-hover:flex'"
    >
      <IconButton
        :icon="Trash2"
        variant="danger"
        size="xs"
        aria-label="Remove"
        :loading="removing"
        :disabled="removeDisabled && !removing"
        @click.stop="emit('remove')"
      />
    </span>
  </div>
</template>
