<script setup lang="ts">
import { computed, onMounted, ref, useAttrs, useSlots } from 'vue'
import { Eye, EyeOff } from '@lucide/vue'
import IconButton from './IconButton.vue'

defineOptions({ inheritAttrs: false })

const props = defineProps<{
  modelValue?: string
  placeholder?: string
  /** Take focus on mount: the field a dialog or form opens on. The ring follows real focus. */
  focused?: boolean
  /** Paint the focus ring without holding focus, for a catalog specimen. */
  showFocus?: boolean
  /** A key or password: masked, with a built-in eye to reveal it. */
  secret?: boolean
  /** Height family: lg is 36px, md is 28px, sm is 24px. Match the surface's other controls. */
  size?: 'sm' | 'md' | 'lg'
  /**
   * No frame in any state: the value reads as text in its row, and the caret is the
   * only sign of focus. The hit area stretches to its container's height so it fills
   * the row's content box, and the prefix sits flush with the left edge.
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
const frameAttrs = computed(() => ({
  class: attrs['class'],
  style: attrs['style']
}))
const inputAttrs = computed(
  () => Object.fromEntries(
    Object.entries(attrs).filter(([
      key
    ]) => key !== 'class' && key !== 'style')
  )
)
const inputRef = ref<HTMLInputElement>()
const isFocused = ref(false)

const frameHeightClass = computed(() => {
  if (props.bare) {
    if (props.size === 'sm')
      return 'min-h-6 self-stretch'
    if (props.size === 'lg')
      return 'min-h-9 self-stretch'
    return 'min-h-7 self-stretch'
  }
  if (props.size === 'sm')
    return 'h-6'
  if (props.size === 'lg')
    return 'h-9'
  return 'h-7'
})

function insetPad(edge: 'pl' | 'pr'): string {
  if (props.size === 'sm')
    return `${edge}-2`
  if (props.size === 'lg')
    return `${edge}-3`
  return `${edge}-2.5`
}

const inputPadClass = computed(() => [
  slots['prefix'] ? 'pl-1.5' : props.bare ? 'pl-0' : insetPad('pl'),
  props.secret || slots['suffix']
    ? 'pr-1.5'
    : props.bare ? 'pr-0' : insetPad('pr'),
])

onMounted(() => {
  if (props.focused)
    inputRef.value?.focus()
})

defineExpose({
  focus() {
    inputRef.value?.focus()
  },
  select() {
    inputRef.value?.select()
  },
  el: inputRef,
})
</script>

<template>
  <div
    v-bind="frameAttrs"
    class="flex min-w-0 items-center rounded-md ring-1 transition-[box-shadow,background-color] duration-200 ease-out"
    :class="[
      frameHeightClass,
      attrs['class'] ? '' : 'w-full',
      bare ? 'bg-transparent ring-transparent' : ['bg-surface-raised', showFocus || isFocused ? 'ring-line-focus' : 'ring-line'],
    ]"
    :data-bare="bare ? true : undefined"
    @click="inputRef?.focus()"
  >
    <div
      v-if="slots['prefix']"
      class="flex shrink-0 items-center text-fg-subtle"
      :class="bare ? '' : 'pl-2'"
    >
      <slot name="prefix" />
    </div>
    <input
      ref="inputRef"
      :type="secret && !revealed ? 'password' : 'text'"
      v-bind="inputAttrs"
      :value="modelValue"
      :placeholder="placeholder"
      class="h-full min-w-0 flex-1 bg-transparent text-chrome text-fg outline-none placeholder:text-fg-subtle"
      :class="inputPadClass"
      @input="emit('update:modelValue', ($event.target as HTMLInputElement).value)"
      @focus="isFocused = true"
      @blur="isFocused = false"
    />
    <div v-if="secret" class="flex shrink-0 items-center pr-1">
      <IconButton
        :icon="revealed ? EyeOff : Eye"
        variant="ghost"
        size="xs"
        :aria-label="revealed ? 'Hide' : 'Show'"
        @click.stop="revealed = !revealed"
      />
    </div>
    <div v-if="slots['suffix']" class="flex shrink-0 items-center pr-2">
      <slot name="suffix" />
    </div>
  </div>
</template>
