<script setup lang="ts">
import { computed } from 'vue'
import { checkboxMark, nextCheckbox } from './checkbox'
import { disabledTooltip } from './disabled'
import Tooltip from './Tooltip.vue'

defineOptions({ inheritAttrs: false })

const props = defineProps<{
  label: string
  disabled?: boolean
  /** Why it is disabled, as a tooltip; only read while `disabled`. */
  disabledReason?: string
}>()

const checked = defineModel<boolean>({ required: true })
const partial = defineModel<boolean>('partial', { default: false })
const tooltipContent = computed(() => disabledTooltip(props.disabled, props.disabledReason))

const mark = computed(() => checkboxMark(checked.value, partial.value))

function toggle() {
  if (props.disabled)
    return
  const next = nextCheckbox(checked.value, partial.value)
  checked.value = next.checked
  partial.value = next.partial
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
      class="group inline-flex h-7 cursor-default items-center gap-2"
      :class="disabled ? 'pointer-events-none cursor-not-allowed opacity-40' : ''"
      role="checkbox"
      :tabindex="disabled ? undefined : 0"
      :aria-checked="partial ? 'mixed' : checked"
      :aria-disabled="disabled || undefined"
      @click="toggle"
      @keydown="onKeydown"
    >
      <span class="checkbox-mark" :data-state="mark" />
      <span class="select-none whitespace-nowrap text-chrome text-fg-body">{{ label }}</span>
    </span>
  </Tooltip>
</template>
