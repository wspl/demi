<script setup lang="ts">
import { computed, watch } from 'vue'
import { Check } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Dialog from '@demicodes/web-ui/ui/Dialog.vue'
import InlineError from '@demicodes/web-ui/ui/InlineError.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
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
      error?: string
    }
  | {
      kind: 'verify'
      email: string
      resent?: boolean
      busy?: boolean
      error?: string
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
    label="Change email"
    @close="emit('close')"
  >
    <div class="flex flex-col gap-4 p-5">
      <header class="select-none pr-10">
        <h3 class="text-[15px] font-medium text-fg-emphasis">Change email</h3>
        <p class="mt-0.5 text-[13px] leading-5 text-fg-muted">
          <template v-if="phase.kind === 'form'"
            >You sign in with the new address from now on.</template
          >
          <template v-else-if="phase.kind === 'verify'"
            >A {{ CODE_LENGTH }}-digit code went to {{ phase.email }}. Enter it
            to finish.</template
          >
          <template v-else>Your email is now {{ phase.email }}.</template>
        </p>
      </header>

      <template v-if="phase.kind === 'form'">
        <div
          class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float"
        >
          <SettingsRow label="New email">
            <TextInput
              v-model="email"
              aria-label="New email"
              focused
              :placeholder="phase.currentEmail"
              class="w-48 max-w-full"
              @keydown.enter="submit"
            />
          </SettingsRow>
          <SettingsRow
            label="Current password"
            description="Confirms it is you."
          >
            <TextInput
              v-model="password"
              aria-label="Current password"
              secret
              class="w-48 max-w-full"
              @keydown.enter="submit"
            />
          </SettingsRow>
        </div>
        <InlineError v-if="phase.error" :message="phase.error" />
        <InlineError
          v-else-if="sameAsCurrent"
          message="That is already your email."
        />
      </template>

      <template v-else-if="phase.kind === 'verify'">
        <div
          class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float"
        >
          <SettingsRow label="Code">
            <TextInput
              v-model="code"
              aria-label="Verification code"
              focused
              placeholder="000000"
              class="w-32 font-mono"
              @keydown.enter="verify"
            />
          </SettingsRow>
        </div>
        <InlineError v-if="phase.error" :message="phase.error" />
        <p class="select-none text-[12px] text-fg-subtle">
          <template v-if="phase.resent"
            >A new code went to {{ phase.email }}.</template
          >
          <template v-else>
            Nothing arrived?
            <button
              type="button"
              class="text-on-accent hover:underline"
              :disabled="phase.busy"
              @click="resend"
            >
              Send it again
            </button>
          </template>
        </p>
      </template>

      <div v-else class="flex items-center gap-2 py-2 text-chrome text-fg">
        <Check :size="ICON_PX.in28" class="text-on-success" />
        {{ phase.email }}
      </div>

      <div class="flex justify-end gap-2">
        <Button
          v-if="phase.kind === 'done'"
          variant="primary"
          @click="emit('close')"
          >Done</Button
        >
        <template v-else>
          <Button @click="emit('close')">Cancel</Button>
          <Button
            v-if="phase.kind === 'form'"
            variant="primary"
            :disabled="!canSubmit && !phase.busy"
            :loading="phase.busy"
            @click="submit"
            >Continue</Button
          >
          <Button
            v-else
            variant="primary"
            :disabled="!canVerify && !phase.busy"
            :loading="phase.busy"
            @click="verify"
            >Verify</Button
          >
        </template>
      </div>
    </div>
  </Dialog>
</template>
