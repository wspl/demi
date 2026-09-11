<script setup lang="ts">
import { ref } from 'vue'
import EmailLoginPage from '@demicodes/web-ui/auth/EmailLoginPage.vue'
import ChangeEmailDialog, {
  type ChangeEmailPhase,
} from '@demicodes/web-ui/settings/ChangeEmailDialog.vue'
import ChangePasswordDialog, {
  type ChangePasswordPhase,
} from '@demicodes/web-ui/settings/ChangePasswordDialog.vue'
import ProviderLoginDialog, {
  type ProviderLoginPhase,
} from '@demicodes/web-ui/settings/ProviderLoginDialog.vue'
import ModelDialog from '@demicodes/web-ui/settings/ModelDialog.vue'
import SettingsAccount from '@demicodes/web-ui/settings/SettingsAccount.vue'
import DevicePairingDialog from '@demicodes/web-ui/devices/DevicePairingDialog.vue'
import WorkspaceDialog from '@demicodes/web-ui/hosts/WorkspaceDialog.vue'
import CloudSettings from '@demicodes/web-ui/cloud/CloudSettings.vue'
import type { CloudState } from '@demicodes/web-ui/cloud/types'
import type { PairingPhase } from '@demicodes/web-ui/devices/pairing'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { createGalleryFileHosts } from '../fixtures/files'
import { demoDeviceInstallation } from '../fixtures/device-installation'
import GalleryDialogFrame from './GalleryDialogFrame.vue'
import GallerySection from './GallerySection.vue'
import GallerySpecimen from './GallerySpecimen.vue'

/**
 * Every form failure, pinned on its failed phase. A form shows only what the
 * reader can correct here; a request that failed for another reason is a
 * toast, so the settings pages carry no inline "could not save".
 *
 * The dialogs are shown in flow and stay open; Cancel and Close do nothing
 * here because the point is the failed state, not the flow around it.
 */
const login = { error: 'The email or password is incorrect.', reason: 'expired' as const }
const email: ChangeEmailPhase = {
  kind: 'form',
  currentEmail: 'preview@example.test',
  error: 'This email address is already in use.',
}
const verification: ChangeEmailPhase = {
  kind: 'verify',
  email: 'new@example.test',
  error: 'The verification code expired. Request another code.',
}
const password: ChangePasswordPhase = {
  kind: 'form',
  error: 'The current password is incorrect.',
}
const emailDraft = ref({ email: 'existing@example.test', password: 'preview-only', code: '' })
const verificationDraft = ref({ email: 'new@example.test', password: '', code: '123456' })
const passwordDraft = ref({ current: 'wrong-password', next: 'new-preview-password', confirm: 'new-preview-password' })
const providerLogin: ProviderLoginPhase = {
  kind: 'failed',
  message: 'The provider sign-in request expired. Try signing in again.',
}
const pairing: PairingPhase = {
  kind: 'code',
  error: 'This code is unavailable. Keep the runner open and paste its latest code.',
}
const name = ref('Unsaved display name')
const cloud: CloudState = {
  state: 'running',
  phase: 'failed',
  error: 'The last reset failed; your home files remain saved.',
  systemBytes: 10 * 1024 ** 3,
  homeBytes: 20 * 1024 ** 3,
}
const hosts = createGalleryFileHosts()
</script>

<template>
  <div class="space-y-8">
    <GallerySection
      title="Sign in"
      note="A rejected sign-in keeps both fields and puts the line under them."
    >
      <div class="h-[28rem] overflow-auto rounded-xl border border-line bg-surface">
        <EmailLoginPage
          email="preview@example.test"
          password="preview-only"
          :phase="login"
        />
      </div>
    </GallerySection>
    <GallerySection
      title="Dialogs"
      note="A rejected submission keeps the dialog open with its input. The line sits above the buttons, in the form's width."
    >
      <div class="grid items-start gap-6 lg:grid-cols-2">
        <GallerySpecimen wide variant="Email change · rejected address">
          <GalleryDialogFrame>
            <ChangeEmailDialog
              v-model:draft="emailDraft"
              is-open
              :overlay-store="appOverlayStore"
              :phase="email"
            />
          </GalleryDialogFrame>
        </GallerySpecimen>
        <GallerySpecimen wide variant="Email verification · expired code">
          <GalleryDialogFrame>
            <ChangeEmailDialog
              v-model:draft="verificationDraft"
              is-open
              :overlay-store="appOverlayStore"
              :phase="verification"
            />
          </GalleryDialogFrame>
        </GallerySpecimen>
        <GallerySpecimen wide variant="Password change · rejected credentials">
          <GalleryDialogFrame>
            <ChangePasswordDialog
              v-model:draft="passwordDraft"
              is-open
              :overlay-store="appOverlayStore"
              :phase="password"
            />
          </GalleryDialogFrame>
        </GallerySpecimen>
        <GallerySpecimen wide variant="Provider sign-in · expired or unavailable">
          <GalleryDialogFrame>
            <ProviderLoginDialog
              is-open
              :overlay-store="appOverlayStore"
              vendor-name="Model provider"
              :phase="providerLogin"
            />
          </GalleryDialogFrame>
        </GallerySpecimen>
        <GallerySpecimen wide variant="Device pairing · invalid code">
          <GalleryDialogFrame>
            <DevicePairingDialog
              is-open
              :overlay-store="appOverlayStore"
              :installation="demoDeviceInstallation"
              :phase="pairing"
            />
          </GalleryDialogFrame>
        </GallerySpecimen>
        <GallerySpecimen wide variant="Project creation · device unavailable">
          <GalleryDialogFrame>
            <WorkspaceDialog
              is-open
              :overlay-store="appOverlayStore"
              mode="create"
              :projects="[]"
              :current-project-id="null"
              :devices="[
                { id: 'preview-device', name: 'Preview laptop', online: true },
              ]"
              :source-for="() => hosts[0]!.source"
              message="Could not create the project. The selected device is offline."
            />
          </GalleryDialogFrame>
        </GallerySpecimen>
        <GallerySpecimen wide variant="Model save · rejected values">
          <GalleryDialogFrame>
            <ModelDialog
              is-open
              :overlay-store="appOverlayStore"
              mode="edit"
              :model="{
                id: 'custom-model',
                name: 'Custom model',
                contextWindow: 128000,
                outputLimit: 8000,
                efforts: ['medium'],
                extensions: ['.png'],
                fastTier: null,
              }"
              error="The provider rejected this model id. Check the id against the provider's catalog."
            />
          </GalleryDialogFrame>
        </GallerySpecimen>
      </div>
    </GallerySection>
    <GallerySection
      title="Settings pages"
      note="A save that failed keeps the edited value and says so in one word beside it; the product toasts the reason. A persistent condition is part of the row's description."
    >
      <div class="grid items-start gap-6 xl:grid-cols-2">
        <GallerySpecimen wide variant="Display name · not saved">
          <div class="@container rounded-xl border border-line bg-surface px-8 py-8">
            <SettingsAccount
              v-model:name="name"
              name-save="failed"
              email="preview@example.test"
              :email-phase="{ kind: 'form', currentEmail: 'preview@example.test' }"
              :password-phase="{ kind: 'form' }"
              :overlay-store="appOverlayStore"
            />
          </div>
        </GallerySpecimen>
        <GallerySpecimen wide variant="Cloud · last reset failed">
          <div class="@container rounded-xl border border-line bg-surface px-8 py-8">
            <CloudSettings
              :cloud="cloud"
              :reset-error="cloud.error"
              :overlay-store="appOverlayStore"
            />
          </div>
        </GallerySpecimen>
      </div>
    </GallerySection>
  </div>
</template>
