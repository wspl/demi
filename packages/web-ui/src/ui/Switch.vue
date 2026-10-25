<script setup lang="ts">
import { computed } from 'vue'
import { disabledTooltip } from './disabled'
import { usePhone } from './phone'
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
   * The operation a flip starts (`operations.ts`), such as a plugin's call: while it
   * runs, whenever the switch is drawn, a spinner turns in its knob, as Ant
   * Design's loading switch does, so nothing beside it moves, and a flip starts
   * nothing.
   *
   * The switch keeps showing `modelValue`, the state its owner sends once the call is
   * answered.
   */
  operation?: Operation
  /** Takes the value a flip asks for; a promise it returns is the work of the switch's operation. */
  'onUpdate:modelValue'?: Action<[value: boolean]>
}>(), {
  size: 'md',
})

const tooltipContent = computed(() => disabledTooltip(props.disabled, props.disabledReason))
const phone = usePhone()
/** The knob's spinner leaves a 2 px rim of the knob around it. */
const knobSpinnerPx = computed(() => (phone.value ? 21 : props.size === 'sm' ? 8 : 12))
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
      <span v-if="label" class="select-none text-control-sm text-fg-subtle">{{ label }}</span>
      <!-- A phone draws either size as iOS's switch, 51 × 31 px with a 27 px knob that travels 20. -->
      <span
        class="inline-flex shrink-0 items-center rounded-full transition-colors duration-200 ease-out phone:h-[31px] phone:w-[51px]"
        :class="[
          modelValue ? 'switch-on' : 'bg-track',
          size === 'sm' ? 'h-4 w-7 p-0.5' : 'h-5 w-9 p-0.5',
        ]"
      >
        <span
          class="flex items-center justify-center rounded-full bg-white text-black/50 shadow-sm transition-transform duration-200 ease-out phone:size-[27px]"
          :class="[
            size === 'sm' ? 'size-3' : 'size-4',
            modelValue ? 'translate-x-full phone:translate-x-5' : 'translate-x-0',
          ]"
        >
          <!-- The spinner's size is fixed when it is drawn, so a size of its own draws it again. -->
          <IndeterminateSpinner
            v-if="running"
            :key="knobSpinnerPx"
            :size="knobSpinnerPx"
            :stroke-width="knobSpinnerPx < 10 ? 1.5 : 2"
            aria-hidden="true"
          />
        </span>
      </span>
    </span>
  </Tooltip>
</template>

<style scoped>
.switch-on {
  background: var(--accent-fill);
}
</style>
