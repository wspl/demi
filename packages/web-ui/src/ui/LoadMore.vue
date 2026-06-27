<script setup lang="ts">
import { ref } from 'vue'
import { useIntersectionObserver } from '@vueuse/core'

const props = withDefaults(defineProps<{
  hasMore: boolean
  isLoading: boolean
  loadingLabel?: string
}>(), {
  loadingLabel: 'Loading...',
})

const emit = defineEmits<{
  load: []
}>()

const sentinel = ref<HTMLElement | null>(null)

useIntersectionObserver(sentinel, ([entry]) => {
  if (entry?.isIntersecting && props.hasMore && !props.isLoading) {
    emit('load')
  }
})
</script>

<template>
  <div ref="sentinel">
    <slot v-if="isLoading" name="loading">
      <div class="py-2 text-center text-[12px] text-fg-subtle">{{ loadingLabel }}</div>
    </slot>
  </div>
</template>
