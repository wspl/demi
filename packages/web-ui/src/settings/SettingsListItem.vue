<script setup lang="ts">
import { t } from './locales/text'
import { Trash2 } from '@lucide/vue'
import { pressOnKey } from '@demicodes/web-ui/ui/button-events'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import TruncatedText from '@demicodes/web-ui/ui/TruncatedText.vue'
import { usePhone } from '@demicodes/web-ui/ui/phone'
import type { SentenceText } from '../ui/ui-text'

/**
 * One entry in a SettingsSplit list: its mark (the `leading` slot), the name,
 * a second line when the list needs one to tell its entries apart, and a dot
 * when its state matters. The selected entry reads as chosen only beside its
 * detail: a narrow split shows the list alone, to open from.
 *
 * `select` carries the click, whose ⌘ and Shift a list that selects several
 * reads. On a phone, which has neither, such a list turns on `checking`: each
 * entry shows a checkbox, checked while it is selected, and a tap toggles it.
 */
defineProps<{
  label: string
  /** The second line, cut with an ellipsis; an entry without one keeps the line's room. */
  detail?: string
  selected?: boolean
  /** The list is choosing several on a phone: the entry shows whether it is chosen. */
  checking?: boolean
  /** A dot on the mark's corner. Omit it when there is nothing to say. */
  badge?: 'success' | 'warning' | 'danger' | 'attention'
  /** What the dot means, said when the pointer is over the mark. */
  badgeLabel?: SentenceText
  muted?: boolean
  /**
   * Shows a remove button on hover, on the desktop. A phone has no hover, and a row's end is
   * where a tap meant to open it lands, so there the entry's page removes it instead, at its
   * bottom.
   */
  removable?: boolean
  removing?: boolean
  removeDisabled?: boolean
}>()

const emit = defineEmits<{
  select: [event: MouseEvent]
  remove: []
}>()

const phone = usePhone()
</script>

<template>
  <div
    role="button"
    :aria-pressed="selected"
    tabindex="0"
    class="group flex cursor-default select-none items-center gap-2 rounded-md phone:gap-3 transition-colors duration-200 ease-out"
    :class="[
      detail === undefined ? 'h-8 px-1 phone:h-row' : 'h-11 px-2 phone:h-14',
      selected && checking ? 'bg-active' : selected ? 'hover:bg-hover wide:bg-active' : 'hover:bg-hover',
      muted ? 'opacity-60' : '',
    ]"
    @click="emit('select', $event)"
    @keydown="pressOnKey"
    @keyup="pressOnKey"
  >
    <span v-if="checking" class="checkbox-mark shrink-0" :data-state="selected ? 'checked' : 'unchecked'" aria-hidden="true" />
    <Tooltip
      v-if="$slots.leading"
      tag="span"
      class="relative flex shrink-0 items-center"
      :content="badgeLabel ?? ''"
      :disabled="!badge || !badgeLabel"
    >
      <slot name="leading" />
      <span
        v-if="badge"
        class="absolute -right-px -top-px size-1.5 rounded-full ring-1 ring-(--fill-color)"
        :class="{
          'bg-on-success': badge === 'success',
          'bg-on-warning': badge === 'warning',
          'bg-on-danger': badge === 'danger',
          'bg-on-attention': badge === 'attention',
        }"
      />
    </Tooltip>
    <span class="flex min-w-0 flex-1 flex-col" :class="detail === undefined ? 'leading-4' : ''">
      <TruncatedText
        class="text-setting"
        :class="[
          detail === undefined ? '' : 'leading-5',
          selected ? 'text-fg wide:text-fg-emphasis' : detail === undefined ? 'text-fg' : 'text-fg-emphasis',
        ]"
        :text="label"
      />
      <TruncatedText v-if="detail !== undefined" class="text-detail leading-4 text-fg-subtle" :text="detail" />
    </span>
    <span
      v-if="removable && !phone"
      :class="removing ? 'on-fill flex shrink-0' : 'on-fill hidden shrink-0 group-hover:flex'"
    >
      <IconButton
        :icon="Trash2"
        variant="danger"
        size="xs"
        :aria-label="t('listItem.remove')"
        :loading="removing"
        :disabled="removeDisabled && !removing"
        @click.stop="emit('remove')"
      />
    </span>
  </div>
</template>
