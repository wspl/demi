<script setup lang="ts">
import { onBeforeUnmount, ref, watch } from 'vue'

/**
 * A picture scaled down to fit and never enlarged; a click shows it at its
 * actual size and another fits it again. Transparency shows over a
 * checkerboard.
 */
const props = defineProps<{ src: string; name: string }>()
const emit = defineEmits<{ size: [width: number, height: number]; failed: [] }>()

const image = ref<HTMLImageElement | null>(null)
const actual = ref(false)
watch(() => props.src, () => {
  actual.value = false
})

function loaded(): void {
  const element = image.value
  if (element)
    emit('size', element.naturalWidth, element.naturalHeight)
}

onBeforeUnmount(() => {
  // Removing the element leaves its fetch running; dropping the source ends it.
  image.value?.removeAttribute('src')
})
</script>

<template>
  <!-- Auto margins center a picture that fits and start one that overflows at its corner. -->
  <div class="flex h-full min-h-0 w-full overflow-auto p-4">
    <img
      ref="image"
      :src="src"
      :alt="name"
      class="checkerboard m-auto shrink-0"
      :class="actual ? 'max-w-none cursor-zoom-out' : 'max-h-full max-w-full cursor-zoom-in object-contain'"
      @load="loaded"
      @error="emit('failed')"
      @click="actual = !actual"
    >
  </div>
</template>

<style scoped>
.checkerboard {
  background: repeating-conic-gradient(var(--line) 0% 25%, transparent 0% 50%) 0 0 / 16px 16px;
}
</style>
