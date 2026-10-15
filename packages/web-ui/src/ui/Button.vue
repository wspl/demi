<script setup lang="ts">
import { computed, ref, useAttrs } from 'vue'
import { blockUnavailableButtonEvent, pressOnKey } from './button-events'
import { useButtonIconSpin } from './button-icon-spin'
import { disabledTooltip } from './disabled'
import { useOperationControl, type Operation } from './operations'
import Tooltip from './Tooltip.vue'
import IndeterminateSpinner from './IndeterminateSpinner.vue'
import { useIconPx } from './icon-metrics'
import type { SentenceText } from './ui-text'

defineOptions({ inheritAttrs: false })
const attrs = useAttrs()
const restAttrs = computed(() => {
  const next: Record<string, unknown> = {}
  for (const [key, value] of Object.entries(attrs)) {
    if (key !== 'class' && key !== 'onClick') {
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
    /**
     * The operation the press starts (`operations.ts`): the button shows the
     * spinner while it runs, whenever it is drawn, and a press while its
     * group runs on its target starts nothing. The press's handler returns
     * the operation's promise; it keeps no flag of its own.
     */
    operation?: Operation
    /** Shows the spinner for work the caller's state owns, such as a reset's phases. */
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
const { rotating, onClick: spinOnPress } = useButtonIconSpin(
  () => root.value ? [...root.value.querySelectorAll(':scope > svg')] : [],
  props,
  () => emit('spinEnd'),
)
// The press starts the button's operation; a press while its group runs starts nothing.
const { running, blocked, press } = useOperationControl(() => props.operation, false)
const working = computed(() => props.loading === true || running.value)
const unavailable = computed(() => props.disabled === true || props.loading === true || blocked.value)
/** The spinner takes a leading icon's size, which follows the button's. */
const icons = useIconPx()
const spinnerPx = computed(() => (props.size === 'xs' ? icons.value.in12 : props.size === 'sm' ? icons.value.in20 : icons.value.in24))

defineExpose({
  /** Presses the button as a click does, for a shortcut that stands for it: refused while it is disabled or working. */
  press(): void {
    root.value?.click()
  },
})

function onClick(event: MouseEvent): void {
  spinOnPress()
  press(attrs.onClick, event)
}

// Its heights are the icon buttons' (`base.css`), so a phone draws it at the
// phone's: 28, 36 and 40 px, and the large one 44.
const sizeClass = computed(() => {
  if (props.size === 'lg') {
    return 'h-9 px-3.5 text-chrome phone:h-11'
  }
  if (props.size === 'xs') {
    return 'h-hit-xs px-1.5 text-control-xs phone:px-2.5'
  }
  if (props.size === 'sm') {
    return 'h-hit-sm px-2 text-control-sm phone:px-3'
  }
  return 'h-hit px-2.5 text-chrome phone:px-3.5'
})
</script>

<template>
  <!-- The tip sits on a wrapper so a disabled face can still be hovered. The face itself
       ignores pointer events, so a parent's click does not fire. It is a Tab stop and takes
       the focus when clicked, as a native button does, and shows the focus ring only for the
       keyboard (`web-application.md` § Keyboard); a disabled or loading one is skipped. A
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
      @click.capture="blockUnavailableButtonEvent($event, unavailable)"
      @keydown.capture="
        blockUnavailableButtonEvent($event, unavailable)
      "
      @keydown="pressOnKey"
      @keyup="pressOnKey"
      role="button"
      :tabindex="unavailable ? undefined : 0"
      :data-default-action="variant === 'primary' || undefined"
      :data-drawn-target="size !== 'xs' || undefined"
      class="relative inline-flex w-full cursor-default items-center justify-center gap-1 whitespace-nowrap rounded-md transition-[color,background-color,box-shadow] duration-200 ease-out select-none"
      :aria-disabled="unavailable || undefined"
      :aria-busy="working || undefined"
      :data-loading="working || undefined"
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
        blocked && !working ? 'opacity-40' : '',
      ]"
    >
      <!-- The spinner takes the place of a leading icon, or stands beside the label; the button keeps its look. -->
      <span v-if="working" class="button-spinner flex shrink-0" aria-hidden="true">
        <IndeterminateSpinner :size="spinnerPx" />
      </span>
      <slot />
    </span>
  </Tooltip>
</template>

<style scoped>
/* A leading icon gives its place to the spinner. */
.button-spinner + :deep(svg) {
  display: none;
}

/* On a phone an icon beside the label is a chip's (`PHONE_ICON_PX.chip`),
   whatever size the caller drew it at for the desktop. */
@media (max-width: 767px) {
  :slotted(svg.lucide) {
    width: 16px;
    height: 16px;
  }
}
</style>
