<script setup lang="ts">
import { reactive, ref } from 'vue'
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
import SettingsProvidersPage from '@demicodes/web-ui/settings/SettingsProvidersPage.vue'
import SettingsAccount from '@demicodes/web-ui/settings/SettingsAccount.vue'
import DevicePairingDialog from '@demicodes/web-ui/devices/DevicePairingDialog.vue'
import WorkspaceDialog from '@demicodes/web-ui/hosts/WorkspaceDialog.vue'
import CloudSettings from '@demicodes/web-ui/cloud/CloudSettings.vue'
import type { CloudState } from '@demicodes/web-ui/cloud/types'
import type { PairingPhase } from '@demicodes/web-ui/devices/pairing'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { createGalleryFileHosts } from '../fixtures/files'
import { mockProviders, mockVendors } from '../fixtures/settings'
import { demoDeviceInstallation } from '../fixtures/device-installation'
import GalleryDialogFrame from './GalleryDialogFrame.vue'
import GallerySection from './GallerySection.vue'
import GallerySpecimen from './GallerySpecimen.vue'

/**
 * Every form and dialog failure, pinned on its failed phase. The dialogs are
 * shown in flow and stay open; Cancel and Close do nothing here because the
 * point is the failed state itself, not the flow around it.
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
const providers = reactive(mockProviders())
const selectedProvider = ref(providers.find((item) => item.kind === 'api_key')!.id)
const saveErrors = {
  [selectedProvider.value]: 'Could not save the endpoint. Your changes are retained.',
}
const name = ref('Unsaved display name')
const cloud: CloudState = {
  state: 'running',
  phase: 'failed',
  error: 'Cloud could not restart. Your home files remain saved.',
  systemBytes: 10 * 1024 ** 3,
  homeBytes: 20 * 1024 ** 3,
}
const hosts = createGalleryFileHosts()
async function saveModel(): Promise<void> {
  throw new Error('The model could not be saved. Your input is retained.')
}
</script>

<template>
  <div class="space-y-8">
    <GallerySection
      title="Sign in"
      note="A rejected sign-in keeps both fields and puts InlineError under them."
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
      note="A rejected submission keeps the dialog open with its input. InlineError sits above the buttons, in the form's width."
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
        <GallerySpecimen wide variant="Model save · retained input">
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
              error="Could not save this model. The endpoint returned HTTP 503."
            />
          </GalleryDialogFrame>
        </GallerySpecimen>
      </div>
    </GallerySection>
    <GallerySection
      title="Settings pages"
      note="A save that failed keeps the edited value and puts InlineError with Retry in the row."
    >
      <GallerySpecimen wide variant="Provider configuration · Retry save">
        <div class="max-h-[36rem] overflow-auto rounded-xl border border-line bg-surface">
          <SettingsProvidersPage
            v-model:selected-id="selectedProvider"
            :providers="providers"
            :vendors="mockVendors"
            :overlay-store="appOverlayStore"
            :save-errors="saveErrors"
            :save-model="saveModel"
          />
        </div>
      </GallerySpecimen>
      <div class="grid items-start gap-6 xl:grid-cols-2">
        <GallerySpecimen wide variant="Display name · save failed">
          <SettingsAccount
            v-model:name="name"
            name-save="failed"
            email="preview@example.test"
            :email-phase="{ kind: 'form', currentEmail: 'preview@example.test' }"
            :password-phase="{ kind: 'form' }"
            :overlay-store="appOverlayStore"
          />
        </GallerySpecimen>
        <GallerySpecimen wide variant="Cloud reset · failed with files retained">
          <CloudSettings
            :cloud="cloud"
            :reset-error="cloud.error"
            :overlay-store="appOverlayStore"
          />
        </GallerySpecimen>
      </div>
    </GallerySection>
  </div>
</template>
