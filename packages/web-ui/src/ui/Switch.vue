<script setup lang="ts">
import { computed } from 'vue'
import { disabledTooltip } from './disabled'
import Tooltip from './Tooltip.vue'

defineOptions({ inheritAttrs: false })

const props = withDefaults(defineProps<{
  modelValue: boolean
  label?: string
  size?: 'sm' | 'md'
  disabled?: boolean
  /** Why it is disabled, as a tooltip; only read while `disabled`. */
  disabledReason?: string
}>(), {
  size: 'md',
})

const tooltipContent = computed(() => disabledTooltip(props.disabled, props.disabledReason))

const emit = defineEmits<{
  'update:modelValue': [value: boolean]
}>()

function toggle() {
  if (props.disabled)
    return
  emit('update:modelValue', !props.modelValue)
}

function onKeydown(event: KeyboardEvent) {
  if (props.disabled)
    return
  if (event.key !== ' ' && event.key !== 'Enter')
    return
  event.preventDefault()
  toggle()
}
</script>

<template>
  <Tooltip
    :content="tooltipContent"
    :disabled="!tooltipContent"
    tag="span"
    class="inline-flex"
    :open-delay-ms="80"
  >
    <span
      v-bind="$attrs"
      class="inline-flex cursor-default items-center gap-1.5"
      :class="disabled ? 'pointer-events-none cursor-not-allowed opacity-40' : ''"
      role="switch"
      :tabindex="disabled ? undefined : 0"
      :aria-checked="modelValue"
      :aria-disabled="disabled || undefined"
      @click="toggle"
      @keydown="onKeydown"
    >
      <span v-if="label" class="select-none text-[12px] text-fg-subtle">{{ label }}</span>
      <span
        class="inline-flex shrink-0 items-center rounded-full transition-colors duration-200 ease-out"
        :class="[
          modelValue ? 'switch-on' : 'bg-overlay/10',
          size === 'sm' ? 'h-4 w-7 p-0.5' : 'h-5 w-9 p-0.5',
        ]"
      >
        <span
          class="rounded-full bg-white shadow-sm transition-transform duration-200 ease-out"
          :class="[
            size === 'sm' ? 'size-3' : 'size-4',
            modelValue ? 'translate-x-full' : 'translate-x-0',
          ]"
        />
      </span>
    </span>
  </Tooltip>
</template>

<style scoped>
.switch-on {
  background: var(--accent-fill);
}
</style>
