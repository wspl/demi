<script setup lang="ts">
import { ref } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import ChangeEmailDialog, { type ChangeEmailPhase } from '@demicodes/web-ui/settings/ChangeEmailDialog.vue'
import ChangePasswordDialog, { type ChangePasswordPhase } from '@demicodes/web-ui/settings/ChangePasswordDialog.vue'
import ProviderLoginDialog, { type ProviderLoginPhase } from '@demicodes/web-ui/settings/ProviderLoginDialog.vue'
import AddProviderDialog from '@demicodes/web-ui/settings/AddProviderDialog.vue'
import ModelDialog from '@demicodes/web-ui/settings/ModelDialog.vue'
import AddSkillSourceDialog from '@demicodes/plugin-skills/AddSkillSourceDialog.vue'
import type { AddSourceAnswer } from '@demicodes/plugin-skills/types'
import { WIRE_API_LABELS, type SettingsModelDraft, type SettingsVendor } from '@demicodes/web-ui/settings/types'
import DevicePairingDialog from '@demicodes/web-ui/devices/DevicePairingDialog.vue'
import DeviceRevokeDialog from '@demicodes/web-ui/devices/DeviceRevokeDialog.vue'
import ConfirmDialog from '@demicodes/web-ui/ui/ConfirmDialog.vue'
import ConversationDeleteDialog from '@demicodes/web-ui/sidebar/ConversationDeleteDialog.vue'
import type { PairingPhase } from '@demicodes/web-ui/devices/pairing'
import type { DeviceInstallation } from '@demicodes/web-ui/devices/installation'
import WorkspaceDialog from '@demicodes/web-ui/hosts/WorkspaceDialog.vue'
import type { WorkspaceDevice, WorkspaceHostChoice } from '@demicodes/web-ui/hosts/workspace'
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
import type { HeadlineText } from '@demicodes/web-ui/ui/ui-text'

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
function finish(close: () => void, title: HeadlineText): void {
  close()
  productWould(title)
}

/**
 * The plugin's answer after a beat: an origin with no slash is refused as the
 * plugin refuses it, and the dialog says so under its field; any other is added.
 */
async function addSkillSource(draft: { origin: string }): Promise<AddSourceAnswer> {
  await new Promise((resolve) => window.setTimeout(resolve, 600))
  if (!draft.origin.includes('/')) {
    return { kind: 'refused', message: `"${draft.origin}" is neither owner/repo nor an https URL` }
  }
  productWould(`Add the Skill Source ${draft.origin}`)
  return { kind: 'added' }
}

const emailPhases: { variant: string; phase: ChangeEmailPhase }[] = [
  { variant: 'form', phase: { kind: 'form', currentEmail: 'zan@example.com' } },
  { variant: 'form · busy', phase: { kind: 'form', currentEmail: 'zan@example.com', busy: true } },
  { variant: 'verify', phase: { kind: 'verify', email: 'zan@demi.codes' } },
  { variant: 'verify · code resent', phase: { kind: 'verify', email: 'zan@demi.codes', resent: true } },
  { variant: 'done', phase: { kind: 'done', email: 'zan@demi.codes' } },
]
const revokeConfirmations: { variant: string; device: string; projects: string[] }[] = [
  { variant: 'with projects', device: 'zan-mbp', projects: ['demi', 'notes'] },
  { variant: 'without projects', device: 'build-01', projects: [] },
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
  { id: hosts[0]!.id, name: hosts[0]!.label, state: 'online' },
  { id: 'build-01', name: 'build-01', state: 'offline' },
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
  lastHost?: WorkspaceHostChoice
  pending: boolean
  load: 'loading' | 'ready'
}[] = [
  { variant: 'first time: Cloud', devices, pending: false, load: 'ready' },
  {
    variant: 'last chose a device',
    devices,
    lastHost: { kind: 'device', deviceId: hosts[0]!.id },
    pending: false,
    load: 'ready',
  },
  { variant: 'creating', devices, pending: true, load: 'ready' },
  { variant: 'devices loading', devices: [], pending: false, load: 'loading' },
]
/**
 * Each project form's remembered choice, as the product's preference keeps it:
 * a card or a device chosen in one is where its next Open starts.
 */
const projectLastHosts = ref(projectForms.map((form) => form.lastHost))
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
  { variant: 'starting · before the server answers', phase: null, submitted: true, error: null, busy: true },
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
      as the product’s does: Cancel, Close and Done close it and Open brings it
      back, and a step the product takes with a Host or another page says so in
      a toast. The Settings page shows where each one opens from, and the
      Errors page shows their failed phases.
    </p>

    <template v-if="view === 'account'">
      <GallerySection
        title="Change Email"
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
                @submit="(email) => productWould(`Send a Verification Code to ${email}`)"
                @verify="(code) => productWould(`Confirm the New Address with Code ${code}`)"
                @resend="productWould('Send a New Verification Code')"
              />
            </GalleryDialogFrame>
          </GallerySpecimen>
        </div>
      </GallerySection>
      <GallerySection title="Change Password" note="The current password and the new one twice; length and the match are checked in the dialog.">
        <div class="grid items-start gap-6 lg:grid-cols-2">
          <GallerySpecimen v-for="item in passwordPhases" :key="item.variant" wide :variant="item.variant">
            <GalleryDialogFrame v-slot="{ open, close }">
              <ChangePasswordDialog
                :is-open="open"
                :overlay-store="appOverlayStore"
                :phase="item.phase"
                @close="close"
                @submit="productWould('Change the Password')"
              />
            </GalleryDialogFrame>
          </GallerySpecimen>
          <!-- As on a phone: each field drops under its label and fills the row. -->
          <GallerySpecimen variant="form · narrow">
            <div class="w-[340px] max-w-full">
              <GalleryDialogFrame v-slot="{ open, close }">
                <ChangePasswordDialog
                  :is-open="open"
                  :overlay-store="appOverlayStore"
                  :phase="{ kind: 'form' }"
                  @close="close"
                  @submit="productWould('Change the Password')"
                />
              </GalleryDialogFrame>
            </div>
          </GallerySpecimen>
        </div>
      </GallerySection>
    </template>

    <template v-if="view === 'providers'">
      <GallerySection
        title="Confirm Removal"
        note="Removing what the user set up asks first: the title asks, the body says what goes with it, Cancel is the safe answer, and the action is the destructive button, filled red under white text in both schemes."
      >
        <div class="grid items-start gap-6 lg:grid-cols-2">
          <GallerySpecimen wide variant="provider">
            <GalleryDialogFrame v-slot="{ open, close }">
              <ConfirmDialog
                :is-open="open"
                :overlay-store="appOverlayStore"
                title="Remove “OpenAI API”?"
                action="Remove"
                @close="close"
                @confirm="finish(close, 'Remove the Provider')"
              >
                <p>Its API key and its 4 models go with it.</p>
                <p>Conversations that use one of its models keep their history, and their next message needs another model.</p>
              </ConfirmDialog>
            </GalleryDialogFrame>
          </GallerySpecimen>
          <GallerySpecimen wide variant="skill source · what goes, named">
            <GalleryDialogFrame v-slot="{ open, close }">
              <ConfirmDialog
                :is-open="open"
                :overlay-store="appOverlayStore"
                title="Remove “agent-skills”?"
                action="Remove"
                :goes="['web-design-guidelines', 'react-best-practices', 'and 3 more']"
                @close="close"
                @confirm="finish(close, 'Remove the Skill Source')"
              >
                <p>Its skills go with it, and the agent is no longer offered them. You can add the repository again later.</p>
              </ConfirmDialog>
            </GalleryDialogFrame>
          </GallerySpecimen>
        </div>
      </GallerySection>
      <GallerySection title="Add Provider" note="A vendor from the catalog, or a bare endpoint speaking one of the protocols Demi implements.">
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
                @add-endpoint="(wireApi) => finish(close, `Add an Endpoint Speaking ${WIRE_API_LABELS[wireApi]}`)"
                @retry="productWould('Load the Provider Catalog Again')"
              />
            </GalleryDialogFrame>
          </GallerySpecimen>
        </div>
      </GallerySection>
      <GallerySection title="Sign In" note="Each subscription signs in the way its vendor does: a token from a CLI, or a device code confirmed in the browser. The code and the link copy separately, each with its own tick, so the link can go to a browser on another machine.">
        <div class="grid items-start gap-6 lg:grid-cols-2">
          <GallerySpecimen v-for="item in loginPhases" :key="item.variant" wide :variant="item.variant">
            <GalleryDialogFrame v-slot="{ open, close }">
              <ProviderLoginDialog
                :is-open="open"
                :overlay-store="appOverlayStore"
                :vendor-name="item.vendor"
                :phase="item.phase"
                @close="close"
                @submit-token="productWould(`Sign In to ${item.vendor} with the Token`)"
                @open="(url) => productWould(`Open ${url} in a New Browser Tab`)"
                @retry="productWould(`Start the ${item.vendor} Sign-in Again`)"
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
      <GallerySection title="Add Device" note="One flow for a local computer, a remote server or a headless machine: start the runner, paste its pairing code, use the connected device.">
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
                @submit="(code) => productWould(`Pair the Device with Code ${code}`)"
              />
            </GalleryDialogFrame>
          </GallerySpecimen>
        </div>
      </GallerySection>
    </template>

    <template v-if="view === 'devices'">
      <GallerySection title="Revoke Device" note="Asked before a device is revoked. The projects on it go with it, named here; their files and conversations stay. A device without projects is asked about plainly.">
        <div class="grid items-start gap-6 lg:grid-cols-2">
          <GallerySpecimen v-for="item in revokeConfirmations" :key="item.variant" wide :variant="item.variant">
            <GalleryDialogFrame v-slot="{ open, close }">
              <DeviceRevokeDialog
                :is-open="open"
                :overlay-store="appOverlayStore"
                :device="item.device"
                :projects="item.projects"
                @close="close"
                @revoke="finish(close, `Revoke the Device ${item.device}`)"
              />
            </GalleryDialogFrame>
          </GallerySpecimen>
        </div>
      </GallerySection>
    </template>

    <template v-if="view === 'workspace'">
      <GallerySection
        title="Delete Conversations"
        note="Delete in a conversation’s sidebar menu, a selection’s menu or an archived row asks first: the title names the one conversation or counts several, the body says that their messages and files go and that this cannot be undone, and that the files they changed in their projects stay. Delete is the destructive button, and Cancel the safe answer."
      >
        <div class="grid items-start gap-6 lg:grid-cols-2">
          <GallerySpecimen
            v-for="item in [
              { variant: 'one conversation', titles: ['Fix the login test'] },
              { variant: 'a selection of three', titles: ['Fix the login test', 'Release notes', 'Upgrade Vite'] },
            ]"
            :key="item.variant"
            wide
            :variant="item.variant"
          >
            <GalleryDialogFrame v-slot="{ open, close }">
              <ConversationDeleteDialog
                :is-open="open"
                :overlay-store="appOverlayStore"
                :titles="item.titles"
                @close="close"
                @delete="finish(close, item.titles.length === 1 ? 'Delete the Conversation' : `Delete ${item.titles.length} Conversations`)"
              />
            </GalleryDialogFrame>
          </GallerySpecimen>
        </div>
      </GallerySection>
      <GallerySection title="New Project" note="A project on the Cloud or a device. The first time it opens on the Cloud; after that on the kind and the device chosen last, which each specimen remembers across Close and Open as the product’s preference does. Switching between existing projects is the sidebar’s “Move To” and the header’s workspace control, not a dialog. The directory completes from the device’s folders as it is typed: the folder before the caret lists under the field, filtered fuzzily by what follows its last slash; ↓ and ↑ highlight a row, Tab or Enter completes it and the menu goes on into it, Escape puts the menu away.">
        <div class="grid items-start gap-6 lg:grid-cols-2">
          <GallerySpecimen v-for="(form, index) in projectForms" :key="form.variant" wide :variant="form.variant">
            <GalleryDialogFrame v-slot="{ open, close }">
              <WorkspaceDialog
                :is-open="open"
                :overlay-store="appOverlayStore"
                :devices="form.devices"
                :last-host="projectLastHosts[index]"
                :pending="form.pending"
                :load="form.load"
                :source-for="sourceFor"
                :places-for="placesFor"
                @close="close"
                @create="(draft) => finish(close, draft.kind === 'cloud' ? `Create the Cloud Project ${draft.name} and Open a New Conversation in It` : `Create the Project at ${draft.path} and Open a New Conversation in It`)"
                @choose="(choice) => (projectLastHosts[index] = choice)"
                @connect-device="productWould('Add Device')"
                @retry="productWould('Load the Devices Again')"
              />
            </GalleryDialogFrame>
          </GallerySpecimen>
        </div>
      </GallerySection>
      <GallerySection title="File Browser" note="Choosing a folder or a file on a device, from the composer’s remote attachment and the new-project form’s Browse.">
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
                @select="(path) => finish(close, `Use ${path} as the Project’s Folder`)"
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
                @select="(path) => finish(close, `Attach ${path} to the Message`)"
              />
            </GalleryDialogFrame>
          </GallerySpecimen>
        </div>
      </GallerySection>
    </template>

    <template v-if="view === 'cloud'">
      <GallerySection title="Reset Cloud Environment" note="Confirmed, then followed step by step: Reset and Retry Reset turn into the spinner at once. A failed reset stays in the dialog with Retry Reset, its reason in words.">
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
                @reset="productWould('Reset the Cloud Environment')"
              />
            </GalleryDialogFrame>
          </GallerySpecimen>
        </div>
      </GallerySection>
    </template>

    <template v-if="view === 'skills'">
      <GallerySection title="Add Skill Source" note="A Git repository whose SKILL.md files become a pack.">
        <GalleryDialogFrame v-slot="{ open, close }" class="max-w-md">
          <AddSkillSourceDialog
            :is-open="open"
            :overlay-store="appOverlayStore"
            :add-source="addSkillSource"
            @close="close"
          />
        </GalleryDialogFrame>
      </GallerySection>
    </template>
  </div>
</template>
