<script setup lang="ts">
import { ref, useSlots } from 'vue'

defineOptions({ inheritAttrs: false })

const props = defineProps<{
  modelValue?: string
  placeholder?: string
  focused?: boolean
}>()

const emit = defineEmits<{
  'update:modelValue': [value: string]
}>()

const slots = useSlots()
const inputRef = ref<HTMLInputElement>()
const isFocused = ref(false)
const startFocused = ref(props.focused ?? false)

defineExpose({
  focus() { inputRef.value?.focus() },
  select() { inputRef.value?.select() },
  el: inputRef,
})
</script>

<template>
  <div
    class="flex h-7 w-full items-center rounded-md bg-surface-raised ring-1 transition-shadow duration-200 ease-out"
    :class="startFocused || isFocused ? 'ring-line-focus' : 'ring-line'"
    @click="inputRef?.focus()"
  >
    <input
      ref="inputRef"
      v-bind="$attrs"
      type="text"
      :value="modelValue"
      :placeholder="placeholder"
      class="h-full min-w-0 flex-1 bg-transparent px-2.5 text-chrome text-fg outline-none placeholder:text-fg-subtle"
      @input="emit('update:modelValue', ($event.target as HTMLInputElement).value)"
      @focus="isFocused = true"
      @blur="isFocused = false; startFocused = false"
    />
    <div v-if="slots['suffix']" class="flex shrink-0 items-center pr-2">
      <slot name="suffix" />
    </div>
  </div>
</template>
