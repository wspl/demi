<script setup lang="ts">
import { computed, ref } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Avatar from '../ui/Avatar.vue'
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
import type { SentenceText } from '../ui/ui-text'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsPasskeys from './SettingsPasskeys.vue'
import SettingsSessions from './SettingsSessions.vue'
import type { SettingsPasskeyHost, SettingsSessionHost } from './types'
import SettingsRow from './SettingsRow.vue'

/**
 * Who you are here. The credential dialogs open from their rows; the host drives
 * their phases (the server checks the password and sends the code) and the page
 * reports what the user typed. The Passkeys group shows where the host offers
 * passkeys, and adds, renames and removes them through the host's functions;
 * Signed-In Browsers lists the account's sessions and signs others out.
 */
const props = defineProps<{
  overlayStore: OverlayStore
  /** The account id, which the avatar's tone follows. */
  accountId: string
  email: string
  emailVerified?: boolean
  /**
   * Why the email cannot be changed, when it cannot: the server sends no
   * mail, which the change needs for its code. The row says so from the
   * start and offers no change.
   */
  emailUnavailable?: SentenceText
  /** When the password last changed, formatted by the host. */
  passwordChanged?: string | null
  emailPhase: ChangeEmailPhase
  passwordPhase: ChangePasswordPhase
  /** The account's passkeys; none where this server or browser offers none. */
  passkeys?: SettingsPasskeyHost
  /** The browsers signed in to the account. */
  sessions: SettingsSessionHost
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


/** Why the name in the field is not saved; it stays in the field until it is fixed. */
const nameProblem = ref<string | null>(null)
function checkName(value: string): string | null {
  return value.trim() ? null : 'Enter a name. Your messages and the sidebar show it.'
}
</script>

<template>
  <SettingsPage title="Account" description="Who you are here.">
    <SettingsGroup title="Profile">
      <SettingsRow
        label="Avatar"
        description="Your initials, until pictures arrive."
      >
        <Avatar :account-id="accountId" :name="name" :email="email" :size="32" />
      </SettingsRow>
      <SettingsRow label="Display name">
        <template v-if="nameProblem" #detail>
          <InlineError :message="nameProblem" />
        </template>
        <CommitTextInput
          :model-value="name"
          aria-label="Display name"
          :validate="checkName"
          @problem="nameProblem = $event"
          @commit="name = $event.trim()"
          maxlength="50"
          class="w-56 max-w-full"
        />
      </SettingsRow>
      <SettingsRow label="Email" :description="emailUnavailable">
        <template v-if="emailVerified" #tags
          ><Tag tone="success">Verified</Tag></template
        >
        <span class="text-chrome text-fg-muted">{{ email }}</span>
        <Button
          size="sm"
          aria-label="Change email"
          :disabled="!!emailUnavailable"
          :disabled-reason="emailUnavailable"
          @click="emit('changeEmail')"
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
      :email="email"
      @close="passwordOpen = false"
      @submit="(current, next) => emit('submitPassword', current, next)"
    />
    <SettingsPasskeys v-if="passkeys" :host="passkeys" :overlay-store="overlayStore" :email="email" />
    <SettingsSessions :host="sessions" :overlay-store="overlayStore" />
    <SettingsGroup title="Session">
      <SettingsRow
        label="Sign out"
        description="Ends this browser’s session. Conversations stay on the server."
      >
        <Button size="sm" @click="emit('signOut')">Sign Out</Button>
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="Danger Zone">
      <SettingsRow
        label="Delete account"
        description="Removes your account, devices and every conversation. This cannot be undone."
        disabled
        :disabled-reason="IN_DEVELOPMENT"
      >
        <Button size="sm" variant="danger" disabled>Delete Account</Button>
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>
</template>
