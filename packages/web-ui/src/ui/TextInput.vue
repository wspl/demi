<script setup lang="ts">
import { computed, ref, useAttrs, useSlots } from 'vue'
import { Eye, EyeOff } from '@lucide/vue'
import IconButton from './IconButton.vue'

defineOptions({ inheritAttrs: false })

const props = defineProps<{
  modelValue?: string
  placeholder?: string
  focused?: boolean
  /** A key or password: masked, with a built-in eye to reveal it. */
  secret?: boolean
}>()

const revealed = ref(false)

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
    <div v-if="slots['prefix']" class="flex shrink-0 items-center pl-2 text-fg-subtle">
      <slot name="prefix" />
    </div>
    <input
      ref="inputRef"
      :type="secret && !revealed ? 'password' : 'text'"
      v-bind="inputAttrs"
      :value="modelValue"
      :placeholder="placeholder"
      class="h-full min-w-0 flex-1 bg-transparent text-chrome text-fg outline-none placeholder:text-fg-subtle"
      :class="[slots['prefix'] ? 'pl-1.5' : 'pl-2.5', secret || slots['suffix'] ? 'pr-1.5' : 'pr-2.5']"
      @input="emit('update:modelValue', ($event.target as HTMLInputElement).value)"
      @focus="isFocused = true"
      @blur="isFocused = false; startFocused = false"
    />
    <div v-if="secret" class="flex shrink-0 items-center pr-1">
      <IconButton :icon="revealed ? EyeOff : Eye" variant="ghost" size="xs" :aria-label="revealed ? 'Hide' : 'Show'" @click.stop="revealed = !revealed" />
    </div>
    <div v-if="slots['suffix']" class="flex shrink-0 items-center pr-2">
      <slot name="suffix" />
    </div>
  </div>
</template>
