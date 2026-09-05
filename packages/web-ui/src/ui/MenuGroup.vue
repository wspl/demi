<script setup lang="ts">
import MenuDivider from './MenuDivider.vue'

defineProps<{
  label: string
}>()
</script>

<template>
  <div class="menu-group">
    <MenuDivider />
    <div class="px-2 pt-1.5 pb-1 text-[11px] font-medium text-fg-subtle">
      {{ label }}
    </div>
    <slot />
  </div>
</template>
