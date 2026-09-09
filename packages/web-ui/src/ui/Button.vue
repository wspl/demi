<script setup lang="ts">
import { computed, ref, useAttrs } from 'vue'
import { useButtonIconSpin } from './button-icon-spin'
import { disabledTooltip } from './disabled'
import Tooltip from './Tooltip.vue'

defineOptions({ inheritAttrs: false })
const attrs = useAttrs()
const restAttrs = computed(() => {
  const next: Record<string, unknown> = {}
  for (const [key, value] of Object.entries(attrs)) {
    if (key !== 'class')
      next[key] = value
  }
  return next
})

const props = withDefaults(defineProps<{
  size?: 'xs' | 'sm' | 'md' | 'lg'
  variant?: 'default' | 'primary' | 'ghost' | 'danger'
  disabled?: boolean
  /** Why it is disabled, as a tooltip; only read while `disabled`. */
  disabledReason?: string
  pressed?: boolean
  spinning?: boolean
  spinOnClick?: boolean
}>(), {
  size: 'md',
  variant: 'default',
})

const pressed = computed(() => props.pressed === true)
const tooltipContent = computed(() => disabledTooltip(props.disabled, props.disabledReason))
const emit = defineEmits<{ spinEnd: [] }>()
const root = ref<HTMLElement | null>(null)
const { rotating, onClick } = useButtonIconSpin(root, props, () => emit('spinEnd'))

const sizeClass = computed(() => {
  if (props.size === 'lg')
    return 'h-9 px-3.5 text-chrome'
  if (props.size === 'xs')
    return 'h-5 px-1.5 text-[11px]'
  if (props.size === 'sm')
    return 'h-6 px-2 text-[12px]'
  return 'h-7 px-2.5 text-chrome'
})
</script>

<template>
  <!-- The tip sits on a wrapper so a disabled face can still be hovered. The face itself
       ignores pointer events, so a parent's click does not fire. -->
  <Tooltip
    :content="tooltipContent"
    :disabled="!tooltipContent"
    tag="span"
    class="inline-flex"
    :class="attrs.class"
    :open-delay-ms="80"
  >
    <span
      ref="root"
      v-bind="restAttrs"
      :data-spinning="rotating || undefined"
      @click="onClick"
      role="button"
      class="inline-flex w-full cursor-default items-center justify-center gap-1 whitespace-nowrap rounded-md transition-[color,background-color,box-shadow,filter] duration-200 ease-out select-none"
      :aria-disabled="disabled || undefined"
      :data-pressed="!disabled && pressed ? true : undefined"
      :class="[
        sizeClass,
        variant === 'primary'
          ? ['btn-primary font-medium text-white', pressed ? 'brightness-110' : 'hover:brightness-110']
          : variant === 'ghost'
            ? pressed
              ? 'font-normal bg-hover text-fg-body'
              : 'font-normal text-fg-muted hover:bg-hover hover:text-fg-body'
            : variant === 'danger'
              ? 'btn font-medium text-on-danger'
              : ['btn font-medium text-fg-body', pressed ? 'text-fg-emphasis' : 'hover:text-fg-emphasis'],
        disabled ? 'pointer-events-none cursor-not-allowed opacity-40' : '',
      ]"
    >
      <slot />
    </span>
  </Tooltip>
</template>
