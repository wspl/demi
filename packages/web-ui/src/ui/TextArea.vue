<script setup lang="ts">
import { ref } from 'vue'

defineOptions({ inheritAttrs: false })

/** Multi-line text with the TextInput frame. Grows with content between the row bounds. */
withDefaults(defineProps<{
  placeholder?: string
  rows?: number
}>(), {
  rows: 3,
})

const model = defineModel<string>({ default: '' })
const isFocused = ref(false)
</script>

<template>
  <div
    class="flex w-full rounded-md bg-surface-raised ring-1 transition-shadow duration-200 ease-out"
    :class="isFocused ? 'ring-line-focus' : 'ring-line'"
  >
    <textarea
      v-model="model"
      v-bind="$attrs"
      :rows="rows"
      :placeholder="placeholder"
      class="min-h-7 w-full resize-y bg-transparent px-2.5 py-1.5 text-chrome leading-5 text-fg outline-none placeholder:text-fg-subtle"
      @focus="isFocused = true"
      @blur="isFocused = false"
    />
  </div>
</template>
