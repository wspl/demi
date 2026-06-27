<script setup lang="ts">
import { ref, useSlots } from 'vue'

defineProps<{
  modelValue?: string
  placeholder?: string
}>()

const emit = defineEmits<{
  'update:modelValue': [value: string]
}>()

const slots = useSlots()
const inputRef = ref<HTMLInputElement>()
const isFocused = ref(false)

defineExpose({
  focus() { inputRef.value?.focus() },
  select() { inputRef.value?.select() },
  el: inputRef,
})
</script>

<template>
  <div
    class="flex h-[26px] w-full items-center rounded bg-surface-raised ring-1 transition-shadow"
    :class="isFocused ? 'ring-line-focus' : 'ring-line'"
    @click="inputRef?.focus()"
  >
    <input
      ref="inputRef"
      type="text"
      :value="modelValue"
      :placeholder="placeholder"
      class="h-full min-w-0 flex-1 bg-transparent px-2 text-xs text-fg outline-none placeholder:text-fg-subtle"
      @input="emit('update:modelValue', ($event.target as HTMLInputElement).value)"
      @focus="isFocused = true"
      @blur="isFocused = false"
    />
    <div v-if="slots['suffix']" class="flex shrink-0 items-center pr-2">
      <slot name="suffix" />
    </div>
  </div>
</template>
