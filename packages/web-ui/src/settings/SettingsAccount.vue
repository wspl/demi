<script setup lang="ts">
import { computed } from 'vue'
import { accountInitial } from '../auth/account-display'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import InlineError from '../ui/InlineError.vue'
import Tag from '../ui/Tag.vue'
import CommitTextInput from '../ui/CommitTextInput.vue'
import ChangeEmailDialog, {
  type ChangeEmailDialogDraft,
  type ChangeEmailPhase,
} from './ChangeEmailDialog.vue'
import ChangePasswordDialog, {
  type ChangePasswordDialogDraft,
  type ChangePasswordPhase,
} from './ChangePasswordDialog.vue'
import { IN_DEVELOPMENT } from '../ui/disabled'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'

/**
 * Who you are here. The credential dialogs open from their rows; the host drives
 * their phases (the server checks the password and sends the code) and the page
 * reports what the user typed.
 */
const props = defineProps<{
  overlayStore: OverlayStore
  nameSave?: 'idle' | 'saving' | 'saved' | 'failed'
  email: string
  emailVerified?: boolean
  /** When the password last changed, formatted by the host. */
  passwordChanged?: string | null
  emailPhase: ChangeEmailPhase
  passwordPhase: ChangePasswordPhase
}>()

const emailDraft = defineModel<ChangeEmailDialogDraft>('emailDraft', {
  default: () => ({ email: '', password: '', code: '' }),
})
const passwordDraft = defineModel<ChangePasswordDialogDraft>('passwordDraft', {
  default: () => ({ current: '', next: '', confirm: '' }),
})
const name = defineModel<string>('name', { required: true })
const emailOpen = defineModel<boolean>('emailOpen', { default: false })
const passwordOpen = defineModel<boolean>('passwordOpen', { default: false })

const emit = defineEmits<{
  retryName: []
  /** The row's Change: the host resets the phase before the dialog shows. */
  changeEmail: []
  changePassword: []
  submitEmail: [email: string, password: string]
  verifyEmail: [code: string]
  resendEmail: []
  submitPassword: [current: string, next: string]
  signOut: []
  deleteAccount: []
}>()

const initial = computed(() => accountInitial(name.value, props.email))
</script>

<template>
  <SettingsPage title="Account" description="Who you are here.">
    <SettingsGroup title="Profile">
      <SettingsRow
        label="Avatar"
        description="Your initial, until pictures arrive."
      >
        <span
          class="flex size-8 select-none items-center justify-center rounded-full bg-tint-accent text-[13px] font-medium text-on-accent"
          >{{ initial }}</span
        >
      </SettingsRow>
      <SettingsRow
        label="Display name"
        description="Shown on your messages and in the sidebar."
      >
        <CommitTextInput
          :model-value="name"
          :disabled="nameSave === 'saving'"
          aria-label="Display name"
          @commit="name = $event"
          maxlength="50"
          class="w-56 max-w-full"
        />
        <span
          v-if="nameSave === 'saving' || nameSave === 'saved'"
          class="text-[12px] text-fg-subtle"
          role="status"
          >{{ nameSave === 'saving' ? 'Saving…' : 'Saved' }}</span
        >
        <InlineError
          v-else-if="nameSave === 'failed'"
          message="Could not save the display name."
          action="Retry"
          @action="emit('retryName')"
        />
      </SettingsRow>
      <SettingsRow label="Email">
        <template v-if="emailVerified" #tags
          ><Tag tone="success">Verified</Tag></template
        >
        <span class="text-chrome text-fg-muted">{{ email }}</span>
        <Button size="sm" aria-label="Change email" @click="emit('changeEmail')"
          >Change</Button
        >
      </SettingsRow>
      <SettingsRow
        label="Password"
        :description="
          passwordChanged ? `Last changed ${passwordChanged}.` : undefined
        "
      >
        <Button
          size="sm"
          aria-label="Change password"
          @click="emit('changePassword')"
          >Change</Button
        >
      </SettingsRow>
    </SettingsGroup>
    <ChangeEmailDialog
      v-model:draft="emailDraft"
      :is-open="emailOpen"
      :overlay-store="overlayStore"
      :phase="emailPhase"
      @close="emailOpen = false"
      @submit="(address, password) => emit('submitEmail', address, password)"
      @verify="emit('verifyEmail', $event)"
      @resend="emit('resendEmail')"
    />
    <ChangePasswordDialog
      v-model:draft="passwordDraft"
      :is-open="passwordOpen"
      :overlay-store="overlayStore"
      :phase="passwordPhase"
      @close="passwordOpen = false"
      @submit="(current, next) => emit('submitPassword', current, next)"
    />
    <SettingsGroup title="Session">
      <SettingsRow
        label="Sign out"
        description="Ends this browser's session. Conversations stay on the server."
      >
        <Button size="sm" @click="emit('signOut')">Sign out</Button>
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="Danger zone">
      <SettingsRow
        label="Delete account"
        description="Removes your account, devices and every conversation. This cannot be undone."
        disabled
        :disabled-reason="IN_DEVELOPMENT"
      >
        <Button size="sm" variant="danger" disabled>Delete account</Button>
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>
</template>
