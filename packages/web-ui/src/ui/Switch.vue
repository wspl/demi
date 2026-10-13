<script setup lang="ts">
import { computed } from 'vue'
import { disabledTooltip } from './disabled'
import IndeterminateSpinner from './IndeterminateSpinner.vue'
import { useOperationControl, type Action, type Operation } from './operations'
import Tooltip from './Tooltip.vue'
import type { SentenceText } from './ui-text'

defineOptions({ inheritAttrs: false })

const props = withDefaults(defineProps<{
  modelValue: boolean
  label?: SentenceText
  size?: 'sm' | 'md'
  disabled?: boolean
  /** Why it is disabled, as a tooltip; only read while `disabled`. */
  disabledReason?: SentenceText
  /**
   * The operation a flip starts (`operations.ts`), such as a plugin's call:
   * while it runs, whenever the switch is drawn, a spinner stands beside it
   * and a flip starts nothing. The switch keeps showing `modelValue`, the
   * state its owner sends once the call is answered.
   */
  operation?: Operation
  /** Takes the value a flip asks for; a promise it returns is the work of the switch's operation. */
  'onUpdate:modelValue'?: Action<[value: boolean]>
}>(), {
  size: 'md',
})

const tooltipContent = computed(() => disabledTooltip(props.disabled, props.disabledReason))
// A flip starts the switch's operation; a flip while it runs starts nothing.
const { running, blocked, press } = useOperationControl(() => props.operation, false)
const unavailable = computed(() => props.disabled === true || blocked.value)

function toggle() {
  if (unavailable.value)
    return
  press(props['onUpdate:modelValue'], !props.modelValue)
}

function onKeydown(event: KeyboardEvent) {
  if (unavailable.value)
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
      class="inline-flex cursor-default items-center gap-1.5 rounded-sm"
      :class="disabled ? 'pointer-events-none cursor-not-allowed opacity-40' : ''"
      role="switch"
      :tabindex="unavailable ? undefined : 0"
      :aria-checked="modelValue"
      :aria-disabled="unavailable || undefined"
      :aria-busy="running || undefined"
      @click="toggle"
      @keydown="onKeydown"
    >
      <span v-if="label" class="select-none text-[12px] text-fg-subtle">{{ label }}</span>
      <!-- While its operation runs the switch keeps its look, with the spinner beside it. -->
      <span v-if="running" class="flex shrink-0" aria-hidden="true">
        <IndeterminateSpinner :size="size === 'sm' ? 12 : 14" />
      </span>
      <span
        class="inline-flex shrink-0 items-center rounded-full transition-colors duration-200 ease-out"
        :class="[
          modelValue ? 'switch-on' : 'bg-track',
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
