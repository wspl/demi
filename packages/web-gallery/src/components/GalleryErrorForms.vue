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
import DevicePairingDialog from '@demicodes/web-ui/devices/DevicePairingDialog.vue'
import WorkspaceDialog from '@demicodes/web-ui/hosts/WorkspaceDialog.vue'
import type { WorkspaceHostChoice } from '@demicodes/web-ui/hosts/workspace'
import type { PairingPhase } from '@demicodes/web-ui/devices/pairing'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { createGalleryFileHosts } from '../fixtures/files'
import { demoDeviceInstallation } from '../fixtures/device-installation'
import { productWould } from '../product-would'
import GalleryDialogFrame from './GalleryDialogFrame.vue'
import GallerySection from './GallerySection.vue'
import GallerySpecimen from './GallerySpecimen.vue'

/**
 * Every form failure, pinned on its failed phase. A form shows only what the
 * reader can correct here; a request that failed for another reason (a save,
 * a reset) is a toast, so the settings pages carry nothing inline.
 *
 * The dialogs are shown in flow, each on its failed phase. Cancel and Close
 * close one and Open brings it back; a submission says in a neutral toast
 * what the product would send, and the phase stays, since the point here is
 * the failure.
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
/** Back and Next move the pairing specimen as the product's do; Open puts the failure back. */
const pairingShown = ref<PairingPhase>(pairing)
const hosts = createGalleryFileHosts()
/** The failed project form's remembered choice: Device on the laptop the Create was for. */
const failedProjectHost = ref<WorkspaceHostChoice>({ kind: 'device', deviceId: 'preview-device' })
</script>

<template>
  <div class="space-y-8">
    <GallerySection
      title="Sign In"
      note="A rejected sign-in keeps both fields and puts the line under them."
    >
      <div class="h-[28rem] overflow-auto rounded-xl border border-line bg-surface">
        <EmailLoginPage
          email="preview@example.test"
          password="preview-only"
          :phase="login"
          @submit="(address) => productWould(`Sign In as ${address}`)"
        />
      </div>
    </GallerySection>
    <GallerySection
      title="Dialogs"
      note="A rejected submission keeps the dialog open with its input. The line sits above the buttons, in the form's width."
    >
      <div class="grid items-start gap-6 lg:grid-cols-2">
        <GallerySpecimen wide variant="Email change · rejected address">
          <GalleryDialogFrame v-slot="{ open, close }">
            <ChangeEmailDialog
              v-model:draft="emailDraft"
              :is-open="open"
              :overlay-store="appOverlayStore"
              :phase="email"
              @close="close"
              @submit="(address) => productWould(`Send a Verification Code to ${address}`)"
            />
          </GalleryDialogFrame>
        </GallerySpecimen>
        <GallerySpecimen wide variant="Email verification · expired code">
          <GalleryDialogFrame v-slot="{ open, close }">
            <ChangeEmailDialog
              v-model:draft="verificationDraft"
              :is-open="open"
              :overlay-store="appOverlayStore"
              :phase="verification"
              @close="close"
              @verify="(code) => productWould(`Confirm the New Address with Code ${code}`)"
              @resend="productWould('Send a New Verification Code')"
            />
          </GalleryDialogFrame>
        </GallerySpecimen>
        <GallerySpecimen wide variant="Password change · rejected credentials">
          <GalleryDialogFrame v-slot="{ open, close }">
            <ChangePasswordDialog
              v-model:draft="passwordDraft"
              :is-open="open"
              :overlay-store="appOverlayStore"
              :phase="password"
              @close="close"
              @submit="productWould('Change the Password')"
            />
          </GalleryDialogFrame>
        </GallerySpecimen>
        <GallerySpecimen wide variant="Provider sign-in · expired or unavailable">
          <GalleryDialogFrame v-slot="{ open, close }">
            <ProviderLoginDialog
              :is-open="open"
              :overlay-store="appOverlayStore"
              vendor-name="Model provider"
              :phase="providerLogin"
              @close="close"
              @retry="productWould('Start the Model Provider Sign-in Again')"
              @open="(url) => productWould(`Open ${url} in a New Browser Tab`)"
              @submit-token="productWould('Sign In to Model Provider with the Token')"
            />
          </GalleryDialogFrame>
        </GallerySpecimen>
        <GallerySpecimen wide variant="Device pairing · invalid code">
          <GalleryDialogFrame v-slot="{ open, close }" @reopen="pairingShown = pairing">
            <DevicePairingDialog
              :is-open="open"
              :overlay-store="appOverlayStore"
              :installation="demoDeviceInstallation"
              :phase="pairingShown"
              @close="close"
              @next="pairingShown = { kind: 'code' }"
              @back="pairingShown = { kind: 'setup' }"
              @submit="(code) => productWould(`Pair the Device with Code ${code}`)"
            />
          </GalleryDialogFrame>
        </GallerySpecimen>
        <GallerySpecimen wide variant="Project creation · device unavailable">
          <GalleryDialogFrame v-slot="{ open, close }">
            <WorkspaceDialog
              :is-open="open"
              :overlay-store="appOverlayStore"
              :devices="[
                { id: 'preview-device', name: 'Preview laptop', online: true },
              ]"
              :source-for="() => hosts[0]!.source"
              :last-host="failedProjectHost"
              message="Could not create the project. The selected device is offline."
              @choose="failedProjectHost = $event"
              @close="close"
              @create="(draft) => productWould(draft.kind === 'cloud' ? `Create the Cloud Project ${draft.name}` : `Create the Project at ${draft.path}`)"
              @connect-device="productWould('Connect New Device')"
              @retry="productWould('Load the Devices Again')"
            />
          </GalleryDialogFrame>
        </GallerySpecimen>
        <GallerySpecimen wide variant="Model save · rejected values">
          <GalleryDialogFrame v-slot="{ open, close }">
            <ModelDialog
              :is-open="open"
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
              @close="close"
              @save="(saved) => productWould(`Save ${saved.name || saved.id}`)"
            />
          </GalleryDialogFrame>
        </GallerySpecimen>
      </div>
    </GallerySection>
  </div>
</template>
