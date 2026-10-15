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
import { isEmail } from '../auth/email'
import SettingsRow from './SettingsRow.vue'

/**
 * Changing the account's email: the new address and the current password, then a
 * code sent to the new address proves it is reachable. Owns nothing; the host drives
 * the phase and reports errors in it.
 */
export type ChangeEmailPhase =
  | {
      kind: 'form'
      currentEmail: string
      busy?: boolean
      error?: ShownError
    }
  | {
      kind: 'verify'
      email: string
      resent?: boolean
      busy?: boolean
      error?: ShownError
    }
  | {
      kind: 'done'
      email: string
    }

const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  phase: ChangeEmailPhase
}>()

const emit = defineEmits<{
  close: []
  submit: [email: string, password: string]
  verify: [code: string]
  resend: []
}>()

export interface ChangeEmailDialogDraft {
  email: string
  password: string
  code: string
}
const draft = defineModel<ChangeEmailDialogDraft>('draft', {
  default: () => ({ email: '', password: '', code: '' }),
})
const email = computed({
  get: () => draft.value.email,
  set: (value: string) => {
    draft.value = { ...draft.value, email: value }
  },
})
const password = computed({
  get: () => draft.value.password,
  set: (value: string) => {
    draft.value = { ...draft.value, password: value }
  },
})
const code = computed({
  get: () => draft.value.code,
  set: (value: string) => {
    draft.value = { ...draft.value, code: value }
  },
})

watch(
  () => [props.isOpen, props.phase.kind],
  () => {
    if (props.phase.kind === 'done') {
      password.value = ''
      code.value = ''
      email.value = ''
    }
  },
)

const CODE_LENGTH = 6

const canSubmit = computed(() => {
  if (props.phase.kind !== 'form' || props.phase.busy) {
    return false
  }
  const next = email.value.trim()
  return (
    isEmail(next) &&
    next !== props.phase.currentEmail &&
    password.value.length > 0
  )
})
const sameAsCurrent = computed(
  () =>
    props.phase.kind === 'form' &&
    email.value.trim() === props.phase.currentEmail,
)
const canVerify = computed(
  () =>
    props.phase.kind === 'verify' &&
    !props.phase.busy &&
    code.value.trim().length === CODE_LENGTH,
)

function submit() {
  if (canSubmit.value) {
    emit('submit', email.value.trim(), password.value)
  }
}

function verify() {
  if (canVerify.value) {
    emit('verify', code.value.trim())
  }
}

function resend() {
  if (props.phase.kind !== 'verify' || props.phase.busy) {
    return
  }
  emit('resend')
}
</script>

<template>
  <Dialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    :label="t('changeEmail.title')"
    @close="emit('close')"
  >
    <div class="flex flex-col gap-4 px-5 pt-5">
      <header class="select-none pr-10">
        <h3 class="text-[15px] font-medium text-fg-emphasis">{{ t('changeEmail.title') }}</h3>
        <p class="mt-0.5 text-[13px] leading-5 text-fg-muted">
          <template v-if="phase.kind === 'form'">{{ t('changeEmail.formHint') }}</template>
          <template v-else-if="phase.kind === 'verify'">{{ t('changeEmail.verifyHint', { count: CODE_LENGTH, email: phase.email }) }}</template>
          <template v-else>{{ t('changeEmail.doneHint', { email: phase.email }) }}</template>
        </p>
      </header>

      <template v-if="phase.kind === 'form'">
        <div
          class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float"
        >
          <SettingsRow :label="t('changeEmail.newEmail')">
            <TextInput
              v-model="email"
              :aria-label="t('changeEmail.newEmail')"
              autocomplete="off"
              focused
              :placeholder="phase.currentEmail"
              class="w-48 max-w-full"
            />
          </SettingsRow>
          <SettingsRow :label="t('changeEmail.currentPassword')">
            <TextInput
              v-model="password"
              :aria-label="t('changeEmail.currentPassword')"
              masked="password"
              autocomplete="current-password"
              class="w-48 max-w-full"
            />
          </SettingsRow>
          <!-- Last in the card, so the card's row dividers skip it; it is out of the flow. -->
          <input v-bind="hiddenUsername(phase.currentEmail)" />
        </div>
        <InlineError v-if="phase.error" :message="phase.error.message" :request-id="phase.error.requestId" />
        <InlineError
          v-else-if="sameAsCurrent"
          :message="t('changeEmail.sameAsCurrent')"
        />
      </template>

      <template v-else-if="phase.kind === 'verify'">
        <div
          class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float"
        >
          <SettingsRow :label="t('changeEmail.code')">
            <TextInput
              v-model="code"
              :aria-label="t('changeEmail.codeLabel')"
              focused
              placeholder="000000"
              class="w-32 font-mono"
            />
          </SettingsRow>
        </div>
        <InlineError v-if="phase.error" :message="phase.error.message" :request-id="phase.error.requestId" />
        <p class="select-none text-[12px] text-fg-subtle">
          <template v-if="phase.resent">{{ t('changeEmail.resent', { email: phase.email }) }}</template>
          <template v-else>
            {{ t('changeEmail.nothingArrived') }}
            <button
              type="button"
              class="text-on-accent hover:underline"
              :disabled="phase.busy"
              @click="resend"
            >
              {{ t('changeEmail.sendAgain') }}
            </button>
          </template>
        </p>
      </template>

      <div v-else class="flex items-center gap-2 py-2 text-chrome text-fg">
        <Check :size="ICON_PX.in28" class="text-on-success" />
        {{ phase.email }}
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
          v-if="phase.kind === 'form'"
          variant="primary"
          :disabled="!canSubmit && !phase.busy"
          :loading="phase.busy"
          @click="submit"
          >{{ uiText('common.continue') }}</Button
        >
        <Button
          v-else
          variant="primary"
          :disabled="!canVerify && !phase.busy"
          :loading="phase.busy"
          @click="verify"
          >{{ t('changeEmail.verify') }}</Button
        >
      </template>
    </template>
  </Dialog>
</template>
