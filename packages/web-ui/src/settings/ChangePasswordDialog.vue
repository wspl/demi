<script setup lang="ts">
import { t as uiText } from '../locales/text'
import { t } from './locales/text'
import type { ShownError } from '../infra/errors'
import { computed, watch } from 'vue'
import { Check } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Dialog from '@demicodes/web-ui/ui/Dialog.vue'
import InlineError from '@demicodes/web-ui/ui/InlineError.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import { hiddenUsername } from '@demicodes/web-ui/ui/field-autofill'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import SettingsRow from './SettingsRow.vue'
import {
  PASSWORD_MIN_LENGTH,
  passwordHint,
  passwordTooShort,
  passwordsDiffer,
  differs,
  isTooShort,
} from '../auth/password'

/**
 * Changing the account's password: the current one proves it is you, the new one is
 * typed twice. Length and the match are checked here; whether the current password
 * is right is the host's answer, reported in the phase.
 */
export type ChangePasswordPhase =
  | {
      kind: 'form'
      busy?: boolean
      error?: ShownError
    }
  | { kind: 'done' }

const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  phase: ChangePasswordPhase
  /** The account's email, which the password manager files the new password under. */
  email: string
}>()

const emit = defineEmits<{
  close: []
  submit: [current: string, next: string]
}>()

export interface ChangePasswordDialogDraft {
  current: string
  next: string
  confirm: string
}
const draft = defineModel<ChangePasswordDialogDraft>('draft', {
  default: () => ({ current: '', next: '', confirm: '' }),
})
const current = computed({
  get: () => draft.value.current,
  set: (value: string) => {
    draft.value = { ...draft.value, current: value }
  },
})
const next = computed({
  get: () => draft.value.next,
  set: (value: string) => {
    draft.value = { ...draft.value, next: value }
  },
})
const confirm = computed({
  get: () => draft.value.confirm,
  set: (value: string) => {
    draft.value = { ...draft.value, confirm: value }
  },
})

watch(
  () => [props.isOpen, props.phase.kind],
  () => {
    if (props.phase.kind === 'done') {
      current.value = ''
      next.value = ''
      confirm.value = ''
    }
  },
)

const tooShort = computed(() => isTooShort(next.value))
const mismatch = computed(() => differs(next.value, confirm.value))
const unchanged = computed(
  () => next.value.length > 0 && next.value === current.value,
)
const canSubmit = computed(
  () =>
    props.phase.kind === 'form' &&
    !props.phase.busy &&
    current.value.length > 0 &&
    next.value.length >= PASSWORD_MIN_LENGTH &&
    confirm.value === next.value &&
    !unchanged.value,
)

function submit() {
  if (canSubmit.value) {
    emit('submit', current.value, next.value)
  }
}
</script>

<template>
  <Dialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    :label="t('changePassword.title')"
    @close="emit('close')"
  >
    <div class="flex flex-col gap-4 px-5 pt-5">
      <header class="select-none pr-10">
        <h3 class="text-[15px] font-medium text-fg-emphasis">
          {{ t('changePassword.title') }}
        </h3>
        <p v-if="phase.kind === 'form'" class="mt-0.5 text-[13px] leading-5 text-fg-muted">
          {{ t('changePassword.hint') }}
        </p>
      </header>

      <template v-if="phase.kind === 'form'">
        <div
          class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float"
        >
          <SettingsRow :label="t('changePassword.current')">
            <TextInput
              v-model="current"
              :aria-label="t('changePassword.current')"
              masked="password"
              autocomplete="current-password"
              focused
              class="w-48 max-w-full"
            />
          </SettingsRow>
          <SettingsRow
            :label="t('changePassword.new')"
            :description="passwordHint()"
          >
            <TextInput
              v-model="next"
              :aria-label="t('changePassword.new')"
              masked="password"
              autocomplete="new-password"
              class="w-48 max-w-full"
            />
          </SettingsRow>
          <SettingsRow :label="t('changePassword.confirm')">
            <TextInput
              v-model="confirm"
              :aria-label="t('changePassword.confirm')"
              masked="password"
              autocomplete="new-password"
              class="w-48 max-w-full"
            />
          </SettingsRow>
          <!-- Last in the card, so the card's row dividers skip it; it is out of the flow. -->
          <input v-bind="hiddenUsername(email)" />
        </div>
        <InlineError v-if="phase.error" :message="phase.error.message" :request-id="phase.error.requestId" />
        <InlineError
          v-else-if="tooShort"
          :message="passwordTooShort()"
        />
        <InlineError
          v-else-if="unchanged"
          :message="t('changePassword.unchanged')"
        />
        <InlineError v-else-if="mismatch" :message="passwordsDiffer()" />
      </template>

      <div v-else class="flex items-center gap-2 py-2 text-chrome text-fg">
        <Check :size="ICON_PX.in28" class="text-on-success" />
        {{ t('changePassword.done') }}
      </div>
    </div>
    <template #footer>
      <Button
        v-if="phase.kind === 'done'"
        variant="primary"
        @click="emit('close')"
        >{{ uiText('common.done') }}</Button
      >
      <template v-else>
        <Button @click="emit('close')">{{ uiText('common.cancel') }}</Button>
        <Button
          variant="primary"
          :disabled="!canSubmit && !phase.busy"
          :loading="phase.busy"
          @click="submit"
          >{{ t('changePassword.title') }}</Button
        >
      </template>
    </template>
  </Dialog>
</template>
