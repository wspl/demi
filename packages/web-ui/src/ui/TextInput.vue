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
  /** Height family: md is 28px, sm is 24px. Match the surface's other controls. */
  size?: 'sm' | 'md'
  /**
   * No frame at rest: the value reads as text in its row. The hit area stretches to
   * its container's height so it fills the row's content box, hover shows it, and
   * focus brings the frame back.
   */
  bare?: boolean
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
    class="flex min-w-0 items-center rounded-md ring-1 transition-[box-shadow,background-color] duration-200 ease-out"
    :class="[
      bare ? (size === 'sm' ? 'min-h-6 self-stretch' : 'min-h-7 self-stretch') : size === 'sm' ? 'h-6' : 'h-7',
      attrs['class'] ? '' : 'w-full',
      bare
        ? startFocused || isFocused
          ? 'bg-surface-raised ring-line-focus'
          : 'bg-transparent ring-transparent hover:bg-hover'
        : ['bg-surface-raised', startFocused || isFocused ? 'ring-line-focus' : 'ring-line'],
    ]"
    :data-bare="bare ? true : undefined"
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
      :class="[slots['prefix'] ? 'pl-1.5' : size === 'sm' ? 'pl-2' : 'pl-2.5', secret || slots['suffix'] ? 'pr-1.5' : size === 'sm' ? 'pr-2' : 'pr-2.5']"
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
