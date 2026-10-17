<script setup lang="ts">
import MenuDivider from './MenuDivider.vue'
import MenuItem from './MenuItem.vue'
import type { ContextMenuGroup } from './context-menu'

/**
 * Rows given as data (`ContextMenuEntry`), a divider between each group and
 * the next: a context menu's, and the same rows wherever else they are
 * offered, such as a strip's More menu or a phone's title sheet.
 */
defineProps<{
  groups: readonly ContextMenuGroup[]
}>()
</script>

<template>
  <template v-for="(group, index) in groups" :key="index">
    <MenuDivider v-if="index > 0" />
    <MenuItem
      v-for="entry in group"
      :key="entry.label"
      :icon="entry.icon"
      :label="entry.label"
      :shortcut="entry.shortcut"
      :disabled="entry.disabled"
      :disabled-reason="entry.disabledReason"
      :is-danger="entry.danger"
      :operation="entry.operation"
      :on-select="entry.run"
    >
      <template v-if="entry.submenu" #submenu>
        <component :is="entry.submenu" />
      </template>
    </MenuItem>
  </template>
</template>
