<script setup lang="ts">
import MenuDivider from './MenuDivider.vue'
import type { TitleText } from './ui-text'

defineProps<{
  label: TitleText
}>()
</script>

<template>
  <div class="menu-group">
    <MenuDivider />
    <div
      data-menu-group-label
      class="select-none px-2 pt-1.5 pb-1 text-[11px] font-medium text-fg-subtle"
    >
      {{ label }}
    </div>
    <slot />
  </div>
</template>
