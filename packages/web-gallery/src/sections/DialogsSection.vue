<script setup lang="ts">
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import ChangeEmailDialog, { type ChangeEmailPhase } from '@demicodes/web-ui/settings/ChangeEmailDialog.vue'
import ChangePasswordDialog, { type ChangePasswordPhase } from '@demicodes/web-ui/settings/ChangePasswordDialog.vue'
import ProviderLoginDialog, { type ProviderLoginPhase } from '@demicodes/web-ui/settings/ProviderLoginDialog.vue'
import AddProviderDialog from '@demicodes/web-ui/settings/AddProviderDialog.vue'
import ModelDialog from '@demicodes/web-ui/settings/ModelDialog.vue'
import AddMcpServerDialog from '@demicodes/web-ui/settings/AddMcpServerDialog.vue'
import AddSkillSourceDialog from '@demicodes/web-ui/settings/AddSkillSourceDialog.vue'
import type { SettingsModelDraft } from '@demicodes/web-ui/settings/types'
import DevicePairingDialog from '@demicodes/web-ui/devices/DevicePairingDialog.vue'
import type { PairingPhase } from '@demicodes/web-ui/devices/pairing'
import WorkspaceDialog from '@demicodes/web-ui/hosts/WorkspaceDialog.vue'
import type { WorkspaceDevice, WorkspaceProject } from '@demicodes/web-ui/hosts/workspace'
import FileBrowserDialog from '@demicodes/web-ui/files/FileBrowserDialog.vue'
import CloudResetDialog from '@demicodes/web-ui/cloud/CloudResetDialog.vue'
import type { CloudState } from '@demicodes/web-ui/cloud/types'
import GalleryDialogFrame from '../components/GalleryDialogFrame.vue'
import GallerySection from '../components/GallerySection.vue'
import GallerySpecimen from '../components/GallerySpecimen.vue'
import { mockVendors } from '../fixtures/settings'
import { demoDeviceInstallation } from '../fixtures/device-installation'
import { createGalleryFileHosts } from '../fixtures/files'
import { useGalleryView } from '../gallery-views'

/**
 * Every dialog the product opens, pinned on each of its phases, in flow and
 * without a scrim. This is the acceptance page for dialogs: the Settings page
 * shows where they open from, the Errors page shows their failed phases.
 */
const { view } = useGalleryView()

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
  { variant: 'Claude Code · done', phase: { kind: 'done', account: 'zan@example.com · Max 5×' } },
]
const codexPhases: { variant: string; phase: ProviderLoginPhase }[] = [
  { variant: 'Codex · starting', phase: { kind: 'starting' } },
  {
    variant: 'Codex · device code',
    phase: { kind: 'device', url: 'https://auth.openai.com/codex/device', code: 'HXRV-7K2M', expiresIn: '10 min' },
  },
  { variant: 'Codex · done', phase: { kind: 'done', account: 'zan@example.com · Plus' } },
]
const grokPhases: { variant: string; phase: ProviderLoginPhase }[] = [
  { variant: 'Grok Build · paste code', phase: { kind: 'code-input', url: 'https://accounts.x.ai/device' } },
  { variant: 'Grok Build · done', phase: { kind: 'done', account: 'zan@example.com · SuperGrok' } },
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
const pairingPhases: { variant: string; phase: PairingPhase }[] = [
  { variant: 'start the runner', phase: { kind: 'setup' } },
  { variant: 'enter the code', phase: { kind: 'code' } },
  { variant: 'pairing', phase: { kind: 'pairing' } },
  { variant: 'connected', phase: { kind: 'done', device: { id: 'demo-device', name: 'zan-mbp' } } },
]
const hosts = createGalleryFileHosts()
const projects: WorkspaceProject[] = [
  { id: 'demi', name: 'demi', host: 'zan-mbp', path: '/Users/zan/Projects/demi' },
  { id: 'assets', name: 'assetsfactory', host: 'build-01', path: '/srv/assetsfactory' },
  { id: 'notes', name: 'notes', host: 'Cloud', path: '/home/demi/notes' },
]
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
      Every dialog the product opens, pinned on each phase it has. Buttons do
      nothing here; the Settings page shows where each one opens from, and the
      Errors page shows their failed phases.
    </p>

    <template v-if="view === 'account'">
      <GallerySection
        title="Change email"
        note="The new address and the current password, then the code that proves the address is reachable."
      >
        <div class="grid items-start gap-6 lg:grid-cols-2">
          <GallerySpecimen v-for="item in emailPhases" :key="item.variant" wide :variant="item.variant">
            <GalleryDialogFrame>
              <ChangeEmailDialog is-open :overlay-store="appOverlayStore" :phase="item.phase" />
            </GalleryDialogFrame>
          </GallerySpecimen>
        </div>
      </GallerySection>
      <GallerySection title="Change password" note="The current password and the new one twice; length and the match are checked in the dialog.">
        <div class="grid items-start gap-6 lg:grid-cols-2">
          <GallerySpecimen v-for="item in passwordPhases" :key="item.variant" wide :variant="item.variant">
            <GalleryDialogFrame>
              <ChangePasswordDialog is-open :overlay-store="appOverlayStore" :phase="item.phase" />
            </GalleryDialogFrame>
          </GallerySpecimen>
        </div>
      </GallerySection>
    </template>

    <template v-if="view === 'providers'">
      <GallerySection title="Add provider" note="A vendor from the catalog, or a bare endpoint speaking one of the protocols Demi implements.">
        <div class="grid items-start gap-6 lg:grid-cols-2">
          <GallerySpecimen wide variant="catalog loaded">
            <GalleryDialogFrame>
              <AddProviderDialog is-open :overlay-store="appOverlayStore" :vendors="mockVendors" load="ready" />
            </GalleryDialogFrame>
          </GallerySpecimen>
          <GallerySpecimen wide variant="catalog loading">
            <GalleryDialogFrame>
              <AddProviderDialog is-open :overlay-store="appOverlayStore" :vendors="[]" load="loading" />
            </GalleryDialogFrame>
          </GallerySpecimen>
        </div>
      </GallerySection>
      <GallerySection title="Sign in" note="Each subscription signs in the way its vendor does: a token from a CLI, a device code confirmed in the browser, or a code pasted back.">
        <div class="grid items-start gap-6 lg:grid-cols-2">
          <GallerySpecimen v-for="item in claudePhases" :key="item.variant" wide :variant="item.variant">
            <GalleryDialogFrame>
              <ProviderLoginDialog is-open :overlay-store="appOverlayStore" vendor-name="Claude Code" :phase="item.phase" />
            </GalleryDialogFrame>
          </GallerySpecimen>
          <GallerySpecimen v-for="item in codexPhases" :key="item.variant" wide :variant="item.variant">
            <GalleryDialogFrame>
              <ProviderLoginDialog is-open :overlay-store="appOverlayStore" vendor-name="Codex" :phase="item.phase" />
            </GalleryDialogFrame>
          </GallerySpecimen>
          <GallerySpecimen v-for="item in grokPhases" :key="item.variant" wide :variant="item.variant">
            <GalleryDialogFrame>
              <ProviderLoginDialog is-open :overlay-store="appOverlayStore" vendor-name="Grok Build" :phase="item.phase" />
            </GalleryDialogFrame>
          </GallerySpecimen>
        </div>
      </GallerySection>
      <GallerySection title="Model" note="A manually configured model: read-only from a catalog, or created and edited for a bare endpoint.">
        <div class="grid items-start gap-6 xl:grid-cols-2">
          <GallerySpecimen wide variant="view">
            <GalleryDialogFrame>
              <ModelDialog is-open :overlay-store="appOverlayStore" mode="view" :model="model" />
            </GalleryDialogFrame>
          </GallerySpecimen>
          <GallerySpecimen wide variant="create">
            <GalleryDialogFrame>
              <ModelDialog is-open :overlay-store="appOverlayStore" mode="create" :model="emptyModel" />
            </GalleryDialogFrame>
          </GallerySpecimen>
          <GallerySpecimen wide variant="edit">
            <GalleryDialogFrame>
              <ModelDialog is-open :overlay-store="appOverlayStore" mode="edit" :model="model" />
            </GalleryDialogFrame>
          </GallerySpecimen>
          <GallerySpecimen wide variant="edit · saving">
            <GalleryDialogFrame>
              <ModelDialog is-open :overlay-store="appOverlayStore" mode="edit" :model="model" pending />
            </GalleryDialogFrame>
          </GallerySpecimen>
        </div>
      </GallerySection>
    </template>

    <template v-if="view === 'devices'">
      <GallerySection title="Add device" note="One flow for a local computer, a remote server or a headless machine: start the runner, paste its pairing code, use the connected device.">
        <div class="grid items-start gap-6 lg:grid-cols-2">
          <GallerySpecimen v-for="item in pairingPhases" :key="item.variant" wide :variant="item.variant">
            <GalleryDialogFrame>
              <DevicePairingDialog is-open :overlay-store="appOverlayStore" :installation="demoDeviceInstallation" :phase="item.phase" />
            </GalleryDialogFrame>
          </GallerySpecimen>
        </div>
      </GallerySection>
    </template>

    <template v-if="view === 'workspace'">
      <GallerySection title="Working environment" note="The project list to switch between, and the form for a new project on a device or the Cloud.">
        <div class="grid items-start gap-6 lg:grid-cols-2">
          <GallerySpecimen wide variant="switch project">
            <GalleryDialogFrame>
              <WorkspaceDialog is-open :overlay-store="appOverlayStore" mode="switch" :projects="projects" current-project-id="demi" :devices="devices" cloud :source-for="sourceFor" :places-for="placesFor" />
            </GalleryDialogFrame>
          </GallerySpecimen>
          <GallerySpecimen wide variant="switch project · locked while a turn runs">
            <GalleryDialogFrame>
              <WorkspaceDialog is-open :overlay-store="appOverlayStore" mode="switch" :projects="projects" current-project-id="demi" :devices="devices" cloud locked :source-for="sourceFor" :places-for="placesFor" />
            </GalleryDialogFrame>
          </GallerySpecimen>
          <GallerySpecimen wide variant="new project · device or Cloud">
            <GalleryDialogFrame>
              <WorkspaceDialog is-open :overlay-store="appOverlayStore" mode="create" :projects="projects" current-project-id="demi" :devices="devices" cloud :source-for="sourceFor" :places-for="placesFor" />
            </GalleryDialogFrame>
          </GallerySpecimen>
          <GallerySpecimen wide variant="new project · devices only">
            <GalleryDialogFrame>
              <WorkspaceDialog is-open :overlay-store="appOverlayStore" mode="create" :projects="projects" current-project-id="demi" :devices="devices" :source-for="sourceFor" :places-for="placesFor" />
            </GalleryDialogFrame>
          </GallerySpecimen>
          <GallerySpecimen wide variant="project list loading">
            <GalleryDialogFrame>
              <WorkspaceDialog is-open :overlay-store="appOverlayStore" mode="switch" :projects="[]" :current-project-id="null" :devices="devices" load="loading" :source-for="sourceFor" :places-for="placesFor" />
            </GalleryDialogFrame>
          </GallerySpecimen>
        </div>
      </GallerySection>
      <GallerySection title="File browser" note="Choosing a folder or a file on a device, from the composer's remote attachment and the new-project form's Browse.">
        <div class="grid items-start gap-6 xl:grid-cols-2">
          <GallerySpecimen wide variant="select folder">
            <GalleryDialogFrame>
              <FileBrowserDialog is-open :overlay-store="appOverlayStore" mode="directory" :source="hosts[0]!.source" :places="hosts[0]!.places" :hosts="hosts" :host-id="hosts[0]!.id" />
            </GalleryDialogFrame>
          </GallerySpecimen>
          <GallerySpecimen wide variant="open file">
            <GalleryDialogFrame>
              <FileBrowserDialog is-open :overlay-store="appOverlayStore" mode="file" :source="hosts[0]!.source" :places="hosts[0]!.places" :hosts="hosts" :host-id="hosts[0]!.id" />
            </GalleryDialogFrame>
          </GallerySpecimen>
        </div>
      </GallerySection>
    </template>

    <template v-if="view === 'cloud'">
      <GallerySection title="Reset Cloud environment" note="Confirmed, then followed step by step. A failed reset stays in the dialog with Retry reset.">
        <div class="grid items-start gap-6 lg:grid-cols-2">
          <GallerySpecimen v-for="item in resetPhases" :key="item.variant" wide :variant="item.variant">
            <GalleryDialogFrame>
              <CloudResetDialog is-open :overlay-store="appOverlayStore" :phase="item.phase" :submitted="item.submitted" :error="item.error" :busy="item.busy" />
            </GalleryDialogFrame>
          </GallerySpecimen>
        </div>
      </GallerySection>
    </template>

    <template v-if="view === 'catalog'">
      <GallerySection title="Add MCP server" note="A stdio command or a remote URL.">
        <GalleryDialogFrame class="max-w-md">
          <AddMcpServerDialog is-open :overlay-store="appOverlayStore" />
        </GalleryDialogFrame>
      </GallerySection>
      <GallerySection title="Add skill source" note="A git origin whose SKILL.md files become a pack.">
        <GalleryDialogFrame class="max-w-md">
          <AddSkillSourceDialog is-open :overlay-store="appOverlayStore" />
        </GalleryDialogFrame>
      </GallerySection>
    </template>
  </div>
</template>
