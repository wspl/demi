<script setup lang="ts">
defineProps<{
  label: string
}>()
</script>

<template>
  <div class="border-t border-line pt-0.5 first:border-t-0 first:pt-0">
    <div class="px-2.5 pt-1.5 pb-1 text-[11px] font-medium text-fg-subtle">
      {{ label }}
    </div>
    <div class="flex flex-col gap-0.5">
      <slot />
    </div>
  </div>
</template>
