<script setup lang="ts">
import { DownLine } from '@mingcute/vue/down'

const props = withDefaults(defineProps<{
  visible: boolean
  bottomOffset?: number
}>(), {
  bottomOffset: 12,
})

const emit = defineEmits<{
  click: []
}>()
</script>

<template>
  <Transition
    enter-active-class="transition duration-200 ease-out"
    leave-active-class="transition duration-200 ease-in"
    enter-from-class="translate-y-2 opacity-0"
    leave-to-class="translate-y-2 opacity-0"
  >
    <div
      v-if="visible"
      class="absolute right-5 z-20 cursor-pointer rounded-full border border-fg-ghost bg-surface-raised p-1.5 text-fg-muted shadow-lg transition-colors hover:border-fg-faint hover:bg-fg-ghost hover:text-fg"
      :style="{ bottom: `${props.bottomOffset}px` }"
      @click="emit('click')"
    >
      <DownLine :size="16" />
    </div>
  </Transition>
</template>
