<script setup lang="ts">
import { computed, ref, useAttrs } from 'vue'
import { blockUnavailableButtonEvent, pressOnKey } from './button-events'
import { useButtonIconSpin } from './button-icon-spin'
import { disabledTooltip } from './disabled'
import Tooltip from './Tooltip.vue'
import IndeterminateSpinner from './IndeterminateSpinner.vue'
import type { SentenceText } from './ui-text'

defineOptions({ inheritAttrs: false })
const attrs = useAttrs()
const restAttrs = computed(() => {
  const next: Record<string, unknown> = {}
  for (const [key, value] of Object.entries(attrs)) {
    if (key !== 'class') {
      next[key] = value
    }
  }
  return next
})

const props = withDefaults(
  defineProps<{
    size?: 'xs' | 'sm' | 'md' | 'lg'
    /**
     * primary: the default button, filled with the accent, which Return
     * presses in a dialog; in a confirmation whose action destroys
     * something, that is Cancel. danger: an action that destroys, red text
     * on a plain face, never the default, as macOS's alerts show it: a
     * confirmation's answer (Remove, Revoke, Reset Environment), or one
     * that then asks first (Delete beside Save, Delete Account).
     */
    variant?: 'default' | 'primary' | 'ghost' | 'danger'
    disabled?: boolean
    loading?: boolean
    /** Why it is disabled, as a tooltip; only read while `disabled`. */
    disabledReason?: SentenceText
    pressed?: boolean
    /** Turns the icon while true, a whole revolution at a time; the turn it is in always finishes. */
    spinning?: boolean
    /** Turns the icon one whole revolution per click. Every button that refreshes, renews or restarts does. */
    spinOnClick?: boolean
  }>(),
  {
    size: 'md',
    variant: 'default',
  },
)

const pressed = computed(() => props.pressed === true)
const tooltipContent = computed(() =>
  disabledTooltip(props.disabled, props.disabledReason),
)
const emit = defineEmits<{ spinEnd: [] }>()
const root = ref<HTMLElement | null>(null)
// The icons a caller puts straight into the button, beside its label, turn.
const { rotating, onClick } = useButtonIconSpin(
  () => root.value ? [...root.value.querySelectorAll(':scope > svg')] : [],
  props,
  () => emit('spinEnd'),
)

const sizeClass = computed(() => {
  if (props.size === 'lg') {
    return 'h-9 px-3.5 text-chrome'
  }
  if (props.size === 'xs') {
    return 'h-5 px-1.5 text-[11px]'
  }
  if (props.size === 'sm') {
    return 'h-6 px-2 text-[12px]'
  }
  return 'h-7 px-2.5 text-chrome'
})
</script>

<template>
  <!-- The tip sits on a wrapper so a disabled face can still be hovered. The face itself
       ignores pointer events, so a parent's click does not fire. It takes the focus when
       clicked or given it, as a native button does, but is no Tab stop, as on macOS. A
       primary button is a dialog's default button: Return presses it, and a dialog without
       a field opens on it (Dialog). -->
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
      @click.capture="blockUnavailableButtonEvent($event, disabled || loading)"
      @keydown.capture="
        blockUnavailableButtonEvent($event, disabled || loading)
      "
      @keydown="pressOnKey"
      role="button"
      :tabindex="disabled ? undefined : -1"
      :data-default-action="variant === 'primary' || undefined"
      class="relative inline-flex w-full cursor-default items-center justify-center gap-1 whitespace-nowrap rounded-md transition-[color,background-color,box-shadow] duration-200 ease-out select-none"
      :aria-disabled="disabled || loading || undefined"
      :aria-busy="loading || undefined"
      :data-loading="loading || undefined"
      :data-pressed="!disabled && pressed ? true : undefined"
      :class="[
        sizeClass,
        variant === 'primary'
          ? 'btn-primary font-medium text-white'
          : variant === 'ghost'
            ? pressed
              ? 'font-normal bg-hover text-fg-body'
              : 'font-normal text-fg-muted hover:bg-hover hover:text-fg-body'
            : variant === 'danger'
              ? 'btn font-medium text-on-danger'
              : [
                  'btn font-medium text-fg-body',
                  pressed ? 'text-fg-emphasis' : 'hover:text-fg-emphasis',
                ],
        disabled ? 'pointer-events-none cursor-not-allowed opacity-40' : '',
      ]"
    >
      <slot />
      <span
        v-if="loading"
        class="button-loading-indicator absolute inset-0 flex items-center justify-center"
        aria-hidden="true"
      >
        <IndeterminateSpinner />
      </span>
    </span>
  </Tooltip>
</template>

<style scoped>
[data-loading] :deep(> :not(.button-loading-indicator)) {
  opacity: 0;
}
[data-loading] {
  color: transparent !important;
}
.button-loading-indicator {
  color: var(--color-fg-muted);
}
.btn-primary .button-loading-indicator {
  color: white;
}
</style>
