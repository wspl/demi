<script setup lang="ts">
import { computed, ref, useAttrs, useSlots } from 'vue'

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
// Sizing classes belong to the frame; everything else (placeholder rules, maxlength) to the input.
const attrs = useAttrs()
const frameAttrs = computed(() => ({ class: attrs['class'], style: attrs['style'] }))
const inputAttrs = computed(() => Object.fromEntries(Object.entries(attrs).filter(([key]) => key !== 'class' && key !== 'style')))
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
    v-bind="frameAttrs"
    class="flex h-7 min-w-0 items-center rounded-md bg-surface-raised ring-1 transition-shadow duration-200 ease-out"
    :class="[attrs['class'] ? '' : 'w-full', startFocused || isFocused ? 'ring-line-focus' : 'ring-line']"
    @click="inputRef?.focus()"
  >
    <input
      ref="inputRef"
      v-bind="inputAttrs"
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
