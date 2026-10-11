<script setup lang="ts">
import { computed, ref } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import CommitTextInput from './CommitTextInput.vue'
import ConfirmDialog from './ConfirmDialog.vue'
import Dropdown from './Dropdown.vue'
import Menu from './Menu.vue'
import MenuItem from './MenuItem.vue'
import { LIMIT_PROBLEM, limitStep, parseLimit, type LimitEvent, type LimitPhase } from './limit-field'
import type { HeadlineText, SentenceText, TitleText } from './ui-text'

/**
 * A limit: a whole number from 1, or Unlimited. A field holds the number,
 * and a menu at the row's end chooses Limited or Unlimited, as Xcode's
 * scheme editor pairs a choice with a count. Unlimited asks first in a
 * dialog whose body is the default slot; its Cancel, the default button,
 * keeps the number. Limited from Unlimited opens an empty field, and
 * leaving it empty stays Unlimited. The field commits on Enter or blur and
 * reports text that is no limit through `problem`, as CommitTextInput does.
 */
const props = defineProps<{
  /** The limit; null for Unlimited. */
  modelValue: number | null
  /** The setting's name, which the field and the menu are named by. */
  label: SentenceText
  overlayStore: OverlayStore
  /** The dialog's question before Unlimited. */
  confirmTitle: HeadlineText
  /** Its button that allows Unlimited. */
  confirmAction: TitleText
}>()

const emit = defineEmits<{
  /** A new limit; null for Unlimited, once the dialog confirmed it. */
  commit: [value: number | null]
  problem: [message: SentenceText | null]
}>()

const phase = ref<LimitPhase>('shown')
const limited = computed(() => props.modelValue !== null || phase.value === 'typing')
const choice = computed(() => (limited.value ? 'Limited' : 'Unlimited'))

function on(event: LimitEvent): void {
  const step = limitStep(props.modelValue, phase.value, event)
  phase.value = step.phase
  if (step.commit !== undefined) {
    emit('problem', null)
    emit('commit', step.commit)
  }
}

/** Empty text is no problem while the field waits for a first number. */
function validate(text: string): SentenceText | null {
  if (props.modelValue === null && !text.trim()) {
    return null
  }
  return parseLimit(text) === null ? LIMIT_PROBLEM : null
}

/** A field left empty while it waits for a first number goes back to Unlimited. */
function leave(event: FocusEvent): void {
  if (event.target instanceof HTMLInputElement && !event.target.value.trim()) {
    on('leaveEmpty')
  }
}

function commitText(text: string): void {
  const limit = parseLimit(text)
  if (limit !== null) {
    emit('commit', limit)
  }
}
</script>

<template>
  <div class="inline-flex min-w-0 items-center gap-2">
    <CommitTextInput
      v-if="limited"
      :model-value="modelValue === null ? '' : String(modelValue)"
      :focused="phase === 'typing'"
      :validate="validate"
      :aria-label="label"
      size="sm"
      inputmode="numeric"
      class="w-16"
      @commit="commitText"
      @problem="emit('problem', $event)"
      @blur="leave"
    />
    <Dropdown
      size="sm"
      variant="default"
      :overlay-store="overlayStore"
      :trigger-label="`${label}: ${choice}`"
    >
      <template #trigger>{{ choice }}</template>
      <template #content="{ close }">
        <Menu>
          <MenuItem
            label="Limited"
            choice
            :is-selected="limited"
            @select="on('chooseLimited'); close()"
          />
          <MenuItem
            label="Unlimited"
            choice
            :is-selected="!limited"
            @select="on('chooseUnlimited'); close()"
          />
        </Menu>
      </template>
    </Dropdown>
    <ConfirmDialog
      :is-open="phase === 'confirming'"
      :overlay-store="overlayStore"
      :title="confirmTitle"
      :action="confirmAction"
      @close="on('cancel')"
      @confirm="on('confirm')"
    >
      <slot />
    </ConfirmDialog>
  </div>
</template>
