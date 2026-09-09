<script setup lang="ts">
import { computed, ref } from 'vue'
import { Check } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Dialog from '@demicodes/web-ui/ui/Dialog.vue'
import InlineError from '@demicodes/web-ui/ui/InlineError.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import SettingsRow from './SettingsRow.vue'

/**
 * Changing the account's password: the current one proves it is you, the new one is
 * typed twice. Length and the match are checked here; whether the current password
 * is right is the host's answer, reported in the phase.
 */
export type ChangePasswordPhase =
  | {
    kind: 'form';
    busy?: boolean;
    error?: string
  }
  | { kind: 'done' }

const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  phase: ChangePasswordPhase
}>()

const emit = defineEmits<{
  close: []
  submit: [current: string, next: string]
}>()

const MIN_LENGTH = 8

const current = ref('')
const next = ref('')
const confirm = ref('')

const tooShort = computed(() => next.value.length > 0 && next.value.length < MIN_LENGTH)
const mismatch = computed(() => confirm.value.length > 0 && confirm.value !== next.value)
const unchanged = computed(() => next.value.length > 0 && next.value === current.value)
const canSubmit = computed(
  () =>
    props.phase.kind === 'form' &&
    !props.phase.busy &&
    current.value.length > 0 &&
    next.value.length >= MIN_LENGTH &&
    confirm.value === next.value &&
    !unchanged.value,
)

function submit() {
  if (canSubmit.value)
    emit('submit', current.value, next.value)
}
</script>

<template>
  <Dialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    label="Change password"
    @close="emit('close')"
  >
    <div class="flex flex-col gap-4 p-5">
      <header class="select-none pr-10">
        <h3 class="text-[15px] font-medium text-fg-emphasis">Change password</h3>
        <p class="mt-0.5 text-[13px] leading-5 text-fg-muted">
          <template v-if="phase.kind === 'form'">Other sessions are signed out once it changes.</template>
          <template v-else>Password changed. Other sessions are signed out.</template>
        </p>
      </header>

      <template v-if="phase.kind === 'form'">
        <div
          class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float"
        >
          <SettingsRow label="Current password">
            <TextInput
              v-model="current"
              secret
              focused
              class="w-48 max-w-full"
              @keydown.enter="submit"
            />
          </SettingsRow>
          <SettingsRow
            label="New password"
            :description="`At least ${MIN_LENGTH} characters.`"
          >
            <TextInput
              v-model="next"
              secret
              class="w-48 max-w-full"
              @keydown.enter="submit"
            />
          </SettingsRow>
          <SettingsRow label="Confirm new password">
            <TextInput
              v-model="confirm"
              secret
              class="w-48 max-w-full"
              @keydown.enter="submit"
            />
          </SettingsRow>
        </div>
        <InlineError v-if="phase.error" :message="phase.error" />
        <InlineError
          v-else-if="tooShort"
          :message="`A password has at least ${MIN_LENGTH} characters.`"
        />
        <InlineError
          v-else-if="unchanged"
          message="That is your current password."
        />
        <InlineError v-else-if="mismatch" message="The two passwords differ." />
      </template>

      <div v-else class="flex items-center gap-2 py-2 text-chrome text-fg">
        <Check :size="ICON_PX.in28" class="text-on-success" />
        Password changed
      </div>

      <div class="flex justify-end gap-2">
        <Button
          v-if="phase.kind === 'done'"
          variant="primary"
          @click="emit('close')"
        >Done</Button>
        <template v-else>
          <Button @click="emit('close')">Cancel</Button>
          <Button
            variant="primary"
            :disabled="!canSubmit"
            @click="submit"
          >{{ phase.busy ? 'Changing…' : 'Change password' }}</Button>
        </template>
      </div>
    </div>
  </Dialog>
</template>
