<script setup lang="ts">
import { ref } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import ChangeEmailDialog, { type ChangeEmailPhase } from '@demicodes/web-ui/settings/ChangeEmailDialog.vue'
import ChangePasswordDialog, { type ChangePasswordPhase } from '@demicodes/web-ui/settings/ChangePasswordDialog.vue'
import ProviderLoginDialog, { type ProviderLoginPhase } from '@demicodes/web-ui/settings/ProviderLoginDialog.vue'
import AddProviderDialog from '@demicodes/web-ui/settings/AddProviderDialog.vue'
import ModelDialog from '@demicodes/web-ui/settings/ModelDialog.vue'
import AddMcpServerDialog from '@demicodes/web-ui/settings/AddMcpServerDialog.vue'
import AddSkillSourceDialog from '@demicodes/web-ui/settings/AddSkillSourceDialog.vue'
import { WIRE_API_LABELS, type SettingsModelDraft, type SettingsVendor } from '@demicodes/web-ui/settings/types'
import DevicePairingDialog from '@demicodes/web-ui/devices/DevicePairingDialog.vue'
import type { PairingPhase } from '@demicodes/web-ui/devices/pairing'
import type { DeviceInstallation } from '@demicodes/web-ui/devices/installation'
import WorkspaceDialog from '@demicodes/web-ui/hosts/WorkspaceDialog.vue'
import type { WorkspaceDevice } from '@demicodes/web-ui/hosts/workspace'
import FileBrowserDialog from '@demicodes/web-ui/files/FileBrowserDialog.vue'
import CloudResetDialog from '@demicodes/web-ui/cloud/CloudResetDialog.vue'
import type { CloudState } from '@demicodes/web-ui/cloud/types'
import GalleryDialogFrame from '../components/GalleryDialogFrame.vue'
import GallerySection from '../components/GallerySection.vue'
import GallerySpecimen from '../components/GallerySpecimen.vue'
import { mockVendors } from '../fixtures/settings'
import { demoDeviceInstallation, developmentDeviceInstallation } from '../fixtures/device-installation'
import { createGalleryFileHosts } from '../fixtures/files'
import { useGalleryView } from '../gallery-views'
import { productWould } from '../product-would'

/**
 * Every dialog the product opens, pinned on each of its phases, in flow and
 * without a scrim. This is the acceptance page for dialogs: the Settings page
 * shows where they open from, the Errors page shows their failed phases.
 *
 * Every control answers as the product's would. Cancel, Close and Done close
 * the dialog and its frame's Open brings it back; a step inside the dialog
 * moves it; a step the product takes with a Host or another page says so in a
 * neutral toast, and the dialog closes where the product's would.
 */
const { view } = useGalleryView()

/** An action that ends the dialog in the product: it closes, and the rest is the product's. */
function finish(close: () => void, title: string): void {
  close()
  productWould(title)
}

const emailPhases: { variant: string; phase: ChangeEmailPhase }[] = [
  { variant: 'form', phase: { kind: 'form', currentEmail: 'zan@example.com' } },
  { variant: 'form · busy', phase: { kind: 'form', currentEmail: 'zan@example.com', busy: true } },
  { variant: 'verify', phase: { kind: 'verify', email: 'zan@demi.codes' } },
  { variant: 'verify · code resent', phase: { kind: 'verify', email: 'zan@demi.codes', resent: true } },
  { variant: 'done', phase: { kind: 'done', email: 'zan@demi.codes' } },
]
const passwordPhases: { variant: string; phase: ChangePasswordPhase }[] = [
  { variant: 'form', phase: { kind: 'form' } },
  { variant: 'form · busy', phase: { kind: 'form', busy: true } },
  { variant: 'done', phase: { kind: 'done' } },
]
const claudePhases: { variant: string; phase: ProviderLoginPhase }[] = [
  {
    variant: 'Claude Code · token',
    phase: {
      kind: 'token',
      command: 'claude setup-token',
      install: { label: 'Get Claude Code', url: 'https://docs.anthropic.com/en/docs/claude-code/setup' },
      prefix: 'sk-ant-oat01-',
    },
  },
  { variant: 'Claude Code · done', phase: { kind: 'done', account: 'zan@example.com · Max 5×', active: true } },
]
const codexPhases: { variant: string; phase: ProviderLoginPhase }[] = [
  { variant: 'Codex · starting', phase: { kind: 'starting' } },
  {
    variant: 'Codex · device code',
    phase: { kind: 'device', url: 'https://auth.openai.com/codex/device', code: 'HXRV-7K2M', expiresIn: '10 min' },
  },
  { variant: 'Codex · done · the first account', phase: { kind: 'done', account: 'zan@example.com · Plus', active: true } },
  { variant: 'Codex · done · added beside another', phase: { kind: 'done', account: 'zan@work.example · Pro', active: false } },
]
const grokPhases: { variant: string; phase: ProviderLoginPhase }[] = [
  {
    variant: 'Grok Build · device code',
    phase: { kind: 'device', url: 'https://accounts.x.ai/device', code: 'QK4P-9TWD', expiresIn: '10 min' },
  },
  { variant: 'Grok Build · done', phase: { kind: 'done', account: 'zan@example.com · SuperGrok', active: true } },
]
const model: SettingsModelDraft = {
  id: 'claude-sonnet-4-5',
  name: 'Claude Sonnet 4.5',
  contextWindow: 200_000,
  outputLimit: 64_000,
  efforts: ['low', 'medium', 'high'],
  extensions: ['.png', '.jpg', '.pdf'],
  fastTier: null,
}
const emptyModel: SettingsModelDraft = {
  id: '',
  name: '',
  contextWindow: null,
  outputLimit: null,
  efforts: [],
  extensions: null,
  fastTier: null,
}
const providerCatalogs: { variant: string; vendors: SettingsVendor[]; load: 'loading' | 'ready' }[] = [
  { variant: 'catalog loaded', vendors: mockVendors, load: 'ready' },
  { variant: 'catalog loading', vendors: [], load: 'loading' },
]
const loginPhases = [
  ...claudePhases.map((item) => ({ ...item, vendor: 'Claude Code' })),
  ...codexPhases.map((item) => ({ ...item, vendor: 'Codex' })),
  ...grokPhases.map((item) => ({ ...item, vendor: 'Grok Build' })),
]
const modelEditors: {
  variant: string
  mode: 'create' | 'edit' | 'view'
  model: SettingsModelDraft
  pending: boolean
}[] = [
  { variant: 'view', mode: 'view', model, pending: false },
  { variant: 'create', mode: 'create', model: emptyModel, pending: false },
  { variant: 'edit', mode: 'edit', model, pending: false },
  { variant: 'edit · saving', mode: 'edit', model, pending: true },
]
const pairingPhases: { variant: string; phase: PairingPhase; installation?: DeviceInstallation }[] = [
  { variant: 'start the runner', phase: { kind: 'setup' } },
  {
    variant: 'start the runner · development backend over http',
    phase: { kind: 'setup' },
    installation: developmentDeviceInstallation,
  },
  { variant: 'enter the code', phase: { kind: 'code' } },
  { variant: 'pairing', phase: { kind: 'pairing' } },
  { variant: 'connected', phase: { kind: 'done', device: { id: 'demo-device', name: 'zan-mbp' } } },
]
/** Each pairing specimen's phase: Next and Back move it, as the product's do; Open puts it back. */
const pairingShown = ref(pairingPhases.map((item) => item.phase))
const hosts = createGalleryFileHosts()
const devices: WorkspaceDevice[] = [
  { id: hosts[0]!.id, name: hosts[0]!.label, online: true },
  { id: 'build-01', name: 'build-01', online: false },
]
function sourceFor(deviceId: string) {
  return (hosts.find((host) => host.id === deviceId) ?? hosts[0]!).source
}
function placesFor(deviceId: string) {
  return (hosts.find((host) => host.id === deviceId) ?? hosts[0]!).places
}
const projectForms: {
  variant: string
  devices: WorkspaceDevice[]
  pending: boolean
  load: 'loading' | 'ready'
}[] = [
  { variant: 'device or Cloud', devices, pending: false, load: 'ready' },
  { variant: 'creating', devices, pending: true, load: 'ready' },
  { variant: 'devices loading', devices: [], pending: false, load: 'loading' },
]
/** The device each file browser shows; choosing another in its address bar hands over that device's files. */
const folderHostId = ref(hosts[0]!.id)
const fileHostId = ref(hosts[0]!.id)
const resetPhases: {
  variant: string
  phase: CloudState['phase']
  submitted: boolean
  error: string | null
  busy: boolean
}[] = [
  { variant: 'confirm', phase: null, submitted: false, error: null, busy: false },
  { variant: 'rebuilding', phase: 'rebuilding', submitted: true, error: null, busy: true },
  { variant: 'ready', phase: 'ready', submitted: true, error: null, busy: false },
  {
    variant: 'failed',
    phase: 'failed',
    submitted: true,
    error: 'The system image could not be fetched. Your home files remain saved.',
    busy: false,
  },
]
</script>

<template>
  <div class="space-y-8">
    <p class="max-w-3xl text-[13px] leading-5 text-fg-muted">
      Every dialog the product opens, pinned on each phase it has. Each answers
      as the product's does: Cancel, Close and Done close it and Open brings it
      back, and a step the product takes with a Host or another page says so in
      a toast. The Settings page shows where each one opens from, and the
      Errors page shows their failed phases.
    </p>

    <template v-if="view === 'account'">
      <GallerySection
        title="Change email"
        note="The new address and the current password, then the code that proves the address is reachable."
      >
        <div class="grid items-start gap-6 lg:grid-cols-2">
          <GallerySpecimen v-for="item in emailPhases" :key="item.variant" wide :variant="item.variant">
            <GalleryDialogFrame v-slot="{ open, close }">
              <ChangeEmailDialog
                :is-open="open"
                :overlay-store="appOverlayStore"
                :phase="item.phase"
                @close="close"
                @submit="(email) => productWould(`Send a verification code to ${email}`)"
                @verify="(code) => productWould(`Confirm the new address with code ${code}`)"
                @resend="productWould('Send a new verification code')"
              />
            </GalleryDialogFrame>
          </GallerySpecimen>
        </div>
      </GallerySection>
      <GallerySection title="Change password" note="The current password and the new one twice; length and the match are checked in the dialog.">
        <div class="grid items-start gap-6 lg:grid-cols-2">
          <GallerySpecimen v-for="item in passwordPhases" :key="item.variant" wide :variant="item.variant">
            <GalleryDialogFrame v-slot="{ open, close }">
              <ChangePasswordDialog
                :is-open="open"
                :overlay-store="appOverlayStore"
                :phase="item.phase"
                @close="close"
                @submit="productWould('Change the password')"
              />
            </GalleryDialogFrame>
          </GallerySpecimen>
        </div>
      </GallerySection>
    </template>

    <template v-if="view === 'providers'">
      <GallerySection title="Add provider" note="A vendor from the catalog, or a bare endpoint speaking one of the protocols Demi implements.">
        <div class="grid items-start gap-6 lg:grid-cols-2">
          <GallerySpecimen
            v-for="catalog in providerCatalogs"
            :key="catalog.variant"
            wide
            :variant="catalog.variant"
          >
            <GalleryDialogFrame v-slot="{ open, close }">
              <AddProviderDialog
                :is-open="open"
                :overlay-store="appOverlayStore"
                :vendors="catalog.vendors"
                :load="catalog.load"
                @close="close"
                @add="(vendor) => finish(close, `Add ${vendor.name}`)"
                @add-endpoint="(wireApi) => finish(close, `Add an endpoint speaking ${WIRE_API_LABELS[wireApi]}`)"
                @retry="productWould('Load the provider catalog again')"
              />
            </GalleryDialogFrame>
          </GallerySpecimen>
        </div>
      </GallerySection>
      <GallerySection title="Sign in" note="Each subscription signs in the way its vendor does: a token from a CLI, or a device code confirmed in the browser. The code and the link copy separately, each with its own tick, so the link can go to a browser on another machine.">
        <div class="grid items-start gap-6 lg:grid-cols-2">
          <GallerySpecimen v-for="item in loginPhases" :key="item.variant" wide :variant="item.variant">
            <GalleryDialogFrame v-slot="{ open, close }">
              <ProviderLoginDialog
                :is-open="open"
                :overlay-store="appOverlayStore"
                :vendor-name="item.vendor"
                :phase="item.phase"
                @close="close"
                @submit-token="productWould(`Sign in to ${item.vendor} with the token`)"
                @open="(url) => productWould(`Open ${url} in a new browser tab`)"
                @retry="productWould(`Start the ${item.vendor} sign-in again`)"
              />
            </GalleryDialogFrame>
          </GallerySpecimen>
        </div>
      </GallerySection>
      <GallerySection title="Model" note="A manually configured model: read-only from a catalog, or created and edited for a bare endpoint.">
        <div class="grid items-start gap-6 xl:grid-cols-2">
          <GallerySpecimen v-for="editor in modelEditors" :key="editor.variant" wide :variant="editor.variant">
            <GalleryDialogFrame v-slot="{ open, close }">
              <ModelDialog
                :is-open="open"
                :overlay-store="appOverlayStore"
                :mode="editor.mode"
                :model="editor.model"
                :pending="editor.pending"
                @close="close"
                @save="(saved) => finish(close, `Save ${saved.name || saved.id}`)"
              />
            </GalleryDialogFrame>
          </GallerySpecimen>
        </div>
      </GallerySection>
    </template>

    <template v-if="view === 'devices'">
      <GallerySection title="Add device" note="One flow for a local computer, a remote server or a headless machine: start the runner, paste its pairing code, use the connected device.">
        <div class="grid items-start gap-6 lg:grid-cols-2">
          <GallerySpecimen v-for="(item, index) in pairingPhases" :key="item.variant" wide :variant="item.variant">
            <GalleryDialogFrame v-slot="{ open, close }" @reopen="pairingShown[index] = item.phase">
              <DevicePairingDialog
                :is-open="open"
                :overlay-store="appOverlayStore"
                :installation="item.installation ?? demoDeviceInstallation"
                :phase="pairingShown[index]!"
                @close="close"
                @next="pairingShown[index] = { kind: 'code' }"
                @back="pairingShown[index] = { kind: 'setup' }"
                @submit="(code) => productWould(`Pair the device with code ${code}`)"
              />
            </GalleryDialogFrame>
          </GallerySpecimen>
        </div>
      </GallerySection>
    </template>

    <template v-if="view === 'workspace'">
      <GallerySection title="New project" note="A project on a device or the Cloud. Switching between existing projects is the sidebar's Move to and the header's workspace control, not a dialog.">
        <div class="grid items-start gap-6 lg:grid-cols-2">
          <GallerySpecimen v-for="form in projectForms" :key="form.variant" wide :variant="form.variant">
            <GalleryDialogFrame v-slot="{ open, close }">
              <WorkspaceDialog
                :is-open="open"
                :overlay-store="appOverlayStore"
                :devices="form.devices"
                :pending="form.pending"
                :load="form.load"
                :source-for="sourceFor"
                :places-for="placesFor"
                @close="close"
                @create="(draft) => finish(close, draft.kind === 'cloud' ? `Create the Cloud project ${draft.name}` : `Create the project at ${draft.path}`)"
                @connect-device="productWould('Connect new device')"
                @retry="productWould('Load the devices again')"
              />
            </GalleryDialogFrame>
          </GallerySpecimen>
        </div>
      </GallerySection>
      <GallerySection title="File browser" note="Choosing a folder or a file on a device, from the composer's remote attachment and the new-project form's Browse.">
        <div class="grid items-start gap-6 xl:grid-cols-2">
          <GallerySpecimen wide variant="select folder">
            <GalleryDialogFrame v-slot="{ open, close }">
              <FileBrowserDialog
                v-model:host-id="folderHostId"
                :is-open="open"
                :overlay-store="appOverlayStore"
                mode="directory"
                :source="sourceFor(folderHostId)"
                :places="placesFor(folderHostId)"
                :hosts="hosts"
                @close="close"
                @select="(path) => finish(close, `Use ${path} as the project's folder`)"
              />
            </GalleryDialogFrame>
          </GallerySpecimen>
          <GallerySpecimen wide variant="open file">
            <GalleryDialogFrame v-slot="{ open, close }">
              <FileBrowserDialog
                v-model:host-id="fileHostId"
                :is-open="open"
                :overlay-store="appOverlayStore"
                mode="file"
                :source="sourceFor(fileHostId)"
                :places="placesFor(fileHostId)"
                :hosts="hosts"
                @close="close"
                @select="(path) => finish(close, `Attach ${path} to the message`)"
              />
            </GalleryDialogFrame>
          </GallerySpecimen>
        </div>
      </GallerySection>
    </template>

    <template v-if="view === 'cloud'">
      <GallerySection title="Reset Cloud environment" note="Confirmed, then followed step by step. A failed reset stays in the dialog with Retry reset.">
        <div class="grid items-start gap-6 lg:grid-cols-2">
          <GallerySpecimen v-for="item in resetPhases" :key="item.variant" wide :variant="item.variant">
            <GalleryDialogFrame v-slot="{ open, close }">
              <CloudResetDialog
                :is-open="open"
                :overlay-store="appOverlayStore"
                :phase="item.phase"
                :submitted="item.submitted"
                :error="item.error"
                :busy="item.busy"
                @close="close"
                @reset="productWould('Reset the Cloud environment')"
              />
            </GalleryDialogFrame>
          </GallerySpecimen>
        </div>
      </GallerySection>
    </template>

    <template v-if="view === 'catalog'">
      <GallerySection title="Add MCP server" note="A stdio command or a remote URL.">
        <GalleryDialogFrame v-slot="{ open, close }" class="max-w-md">
          <AddMcpServerDialog
            :is-open="open"
            :overlay-store="appOverlayStore"
            @close="close"
            @add="(draft) => finish(close, `Add the MCP server ${draft.name}`)"
          />
        </GalleryDialogFrame>
      </GallerySection>
      <GallerySection title="Add skill source" note="A git origin whose SKILL.md files become a pack.">
        <GalleryDialogFrame v-slot="{ open, close }" class="max-w-md">
          <AddSkillSourceDialog
            :is-open="open"
            :overlay-store="appOverlayStore"
            @close="close"
            @add="(draft) => finish(close, `Add the skill source ${draft.origin}`)"
          />
        </GalleryDialogFrame>
      </GallerySection>
    </template>
  </div>
</template>
