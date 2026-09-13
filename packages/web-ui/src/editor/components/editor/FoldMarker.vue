<script setup lang="ts">
import { ChevronDown, ChevronRight } from '@lucide/vue'

defineProps<{
  isOpen: boolean
}>()
</script>

<template>
  <span class="relative flex size-[18px] items-center justify-center rounded-[3px] hover:bg-active">
    <ChevronDown v-if="isOpen" :size="14" class="absolute text-fg-subtle" />
    <ChevronRight v-else :size="14" class="absolute text-fg-subtle" />
  </span>
</template>
