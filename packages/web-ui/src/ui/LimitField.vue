<script setup lang="ts">
import { computed, ref } from 'vue'
import { Gauge, Infinity as InfinityIcon } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import CommitTextInput from './CommitTextInput.vue'
import ConfirmDialog from './ConfirmDialog.vue'
import Dropdown from './Dropdown.vue'
import Menu from './Menu.vue'
import MenuItem from './MenuItem.vue'
import { LIMIT_PROBLEM, limitStep, parseLimit, type LimitEvent, type LimitPhase, type LimitValue } from './limit-field'
import type { HeadlineText, SentenceText, TitleText } from './ui-text'

/**
 * A limit: a whole number from 1, or Unlimited. A field holds the number,
 * and a menu at the row's end chooses Limited or Unlimited, as Xcode's
 * scheme editor pairs a choice with a count. Unlimited asks first in a
 * dialog whose body is the default slot; its Cancel, the default button,
 * keeps the limit. Unlimited hides the field and keeps its number, which
 * Limited shows again as it was. The field commits on Enter or blur and
 * reports text that is no limit through `problem`, as CommitTextInput does.
 */
const props = defineProps<{
  /** The limit: its number, kept while it is lifted, and whether it is. */
  modelValue: LimitValue
  /** The setting's name, which the field and the menu are named by. */
  label: SentenceText
  overlayStore: OverlayStore
  /** The dialog's question before Unlimited. */
  confirmTitle: HeadlineText
  /** Its button that allows Unlimited. */
  confirmAction: TitleText
}>()

const emit = defineEmits<{
  /** A new limit; Unlimited only once the dialog confirmed it. */
  commit: [value: LimitValue]
  problem: [message: SentenceText | null]
}>()

const phase = ref<LimitPhase>('shown')
const limited = computed(() => !props.modelValue.unlimited)
const choice = computed(() => (limited.value ? 'Limited' : 'Unlimited'))

function on(event: LimitEvent): void {
  const step = limitStep(props.modelValue, phase.value, event)
  phase.value = step.phase
  if (step.commit !== undefined) {
    emit('problem', null)
    emit('commit', step.commit)
  }
}

function validate(text: string): SentenceText | null {
  return parseLimit(text) === null ? LIMIT_PROBLEM : null
}

function commitText(text: string): void {
  const limit = parseLimit(text)
  if (limit !== null) {
    emit('commit', { limit, unlimited: false })
  }
}
</script>

<template>
  <div class="inline-flex min-w-0 items-center gap-2">
    <CommitTextInput
      v-if="limited"
      :model-value="String(modelValue.limit)"
      :validate="validate"
      :aria-label="label"
      size="sm"
      inputmode="numeric"
      class="w-16"
      @commit="commitText"
      @problem="emit('problem', $event)"
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
            :icon="Gauge"
            label="Limited"
            choice
            :is-selected="limited"
            @select="on('chooseLimited'); close()"
          />
          <MenuItem
            :icon="InfinityIcon"
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
