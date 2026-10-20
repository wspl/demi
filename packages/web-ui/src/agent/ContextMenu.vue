<script setup lang="ts">
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'

/**
 * The windows a Context choice offers, one choice row each. It serves two
 * places: the model menu's Context row, for the user's limit, and a subagent
 * profile's own Context (`packages/web-ui/src/agent/context-limit.ts`,
 * `crates/agent-server/src/server/tree/profiles.rs`).
 */
defineProps<{
  options: { label: string; selected: boolean }[]
}>()

const emit = defineEmits<{
  /** The option at `index` was chosen. */
  select: [index: number]
}>()
</script>

<template>
  <Menu>
    <MenuItem
      v-for="(option, index) in options"
      :key="option.label"
      :label="option.label"
      choice
      :is-selected="option.selected"
      @select="emit('select', index)"
    />
  </Menu>
</template>
