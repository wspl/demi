<script setup lang="ts">
import { computed, onMounted, ref, useAttrs, useSlots } from 'vue'
import { Eye, EyeOff } from '@lucide/vue'
import { disabledTooltip } from './disabled'
import IconButton from './IconButton.vue'
import Tooltip from './Tooltip.vue'

defineOptions({ inheritAttrs: false })

const props = withDefaults(defineProps<{
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
  disabled?: boolean
  /** Why it is disabled, as a tooltip; only read while `disabled`. */
  disabledReason?: string
}>(), {
  size: 'md',
})

const revealed = ref(false)

const emit = defineEmits<{
  'update:modelValue': [value: string]
}>()

const slots = useSlots()
// Layout classes belong to the outer flex item; native attributes stay on the input.
const attrs = useAttrs()
const layoutAttrs = computed(() => ({
  class: attrs['class'],
  style: attrs['style']
}))
const inputAttrs = computed(
  () => Object.fromEntries(
    Object.entries(attrs).filter(([key]) => key !== 'class' && key !== 'style')
  )
)
const inputRef = ref<HTMLInputElement>()
const isFocused = ref(false)
const tooltipContent = computed(() => disabledTooltip(props.disabled, props.disabledReason))

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

// Keep complete class names visible to Tailwind's source scanner.
const insetPadding = {
  sm: { left: 'pl-2', right: 'pr-2' },
  md: { left: 'pl-2.5', right: 'pr-2.5' },
  lg: { left: 'pl-3', right: 'pr-3' },
} as const

const inputPadClass = computed(() => [
  slots['prefix'] ? 'pl-1.5' : props.bare ? 'pl-0' : insetPadding[props.size].left,
  props.secret || slots['suffix']
    ? 'pr-1.5'
    : props.bare ? 'pr-0' : insetPadding[props.size].right,
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
  <Tooltip
    :content="tooltipContent"
    :disabled="!tooltipContent"
    tag="div"
    v-bind="layoutAttrs"
    class="flex min-w-0"
    :class="[attrs['class'] ? '' : 'w-full', bare ? 'self-stretch' : '']"
    :open-delay-ms="80"
  >
    <div
      class="flex min-w-0 flex-1 items-center rounded-md ring-1 transition-[box-shadow,background-color] duration-200 ease-out"
      :class="[
      frameHeightClass,
      bare ? 'bg-transparent ring-transparent' : ['bg-surface-raised', showFocus || isFocused ? 'ring-line-focus' : 'ring-line'],
      disabled ? 'cursor-not-allowed opacity-40' : '',
    ]"
      :data-bare="bare ? true : undefined"
      @click="!disabled && inputRef?.focus()"
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
        :disabled="disabled || undefined"
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
          :size="size === 'lg' ? 'sm' : 'xs'"
          :disabled="disabled"
          :aria-label="revealed ? 'Hide' : 'Show'"
          @click.stop="revealed = !revealed"
        />
      </div>
      <div v-if="slots['suffix']" class="flex shrink-0 items-center pr-2">
        <slot name="suffix" />
      </div>
    </div>
  </Tooltip>
</template>
