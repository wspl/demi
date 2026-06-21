<script setup lang="ts">
import { CheckFill } from '@mingcute/vue/check'
import { MinimizeLine } from '@mingcute/vue/minimize'

defineProps<{
  state: 'checked' | 'unchecked' | 'partial'
}>()

const emit = defineEmits<{
  toggle: []
}>()
</script>

<template>
  <span
    class="flex size-3.5 shrink-0 cursor-pointer items-center justify-center rounded-[3px] transition-colors"
    :class="state === 'unchecked'
      ? 'bg-fg-subtle/10 hover:bg-fg-subtle/20'
      : 'bg-fg-subtle/20 text-fg hover:bg-fg-subtle/30'"
    @click.stop="emit('toggle')"
  >
    <MinimizeLine v-if="state === 'partial'" class="size-2.5" />
    <CheckFill v-if="state === 'checked'" class="size-2.5" />
  </span>
</template>
