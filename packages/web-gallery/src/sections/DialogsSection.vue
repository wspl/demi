<script setup lang="ts">
import { correction, requestFailure } from '../fixtures/request-failures'
import type { ShownError } from '@demicodes/web-ui/infra/errors'
import { ref } from 'vue'
import { countOf } from '@demicodes/utils'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import ChangeEmailDialog, { type ChangeEmailPhase } from '@demicodes/web-ui/settings/ChangeEmailDialog.vue'
import ChangePasswordDialog, { type ChangePasswordPhase } from '@demicodes/web-ui/settings/ChangePasswordDialog.vue'
import HandoffDialog from '@demicodes/web-ui/auth/HandoffDialog.vue'
import ChoosePasswordWizard from '@demicodes/web-ui/auth/ChoosePasswordWizard.vue'
import PasskeyOffer from '@demicodes/web-ui/auth/PasskeyOffer.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import { galleryHandoff, type HandoffSpecimenStart } from '../fixtures/handoff'
import { slow } from '../fixtures/slow'
import { galleryPasskeyOffer, type PasskeyOfferScript } from '../fixtures/passkey-offer'
import { DEVICE_REPORTS } from '../fixtures/device-reports'
import type { DeviceReport } from '@demicodes/web-ui/auth/device-unlock'
import GalleryDeviceReport from '../components/GalleryDeviceReport.vue'
import { galleryPasswordChoice, type PasswordChoiceScript } from '../fixtures/password-choice'
import ProviderLoginDialog, { type ProviderLoginPhase } from '@demicodes/web-ui/settings/ProviderLoginDialog.vue'
import AddModelDialog from '@demicodes/web-ui/settings/AddModelDialog.vue'
import AddProviderDialog from '@demicodes/web-ui/settings/AddProviderDialog.vue'
import ModelDialog from '@demicodes/web-ui/settings/ModelDialog.vue'
import AddSkillSourceDialog from '@demicodes/plugin-skills/AddSkillSourceDialog.vue'
import type { AddSourceAnswer } from '@demicodes/plugin-skills/types'
import type { SettingsModelDraft, SettingsVendor } from '@demicodes/web-ui/settings/types'
import DevicePairingDialog from '@demicodes/web-ui/devices/DevicePairingDialog.vue'
import DeviceRenameDialog from '@demicodes/web-ui/devices/DeviceRenameDialog.vue'
import DeviceRevokeDialog from '@demicodes/web-ui/devices/DeviceRevokeDialog.vue'
import ConfirmDialog from '@demicodes/web-ui/ui/ConfirmDialog.vue'
import ConversationDeleteDialog from '@demicodes/web-ui/sidebar/ConversationDeleteDialog.vue'
import type { PairingPhase } from '@demicodes/web-ui/devices/pairing'
import type { DeviceInstallCommands } from '@demicodes/web-ui/devices/installation'
import WorkspaceDialog from '@demicodes/web-ui/hosts/WorkspaceDialog.vue'
import type { WorkspaceDevice, WorkspaceHostChoice } from '@demicodes/web-ui/hosts/workspace'
import FileBrowserDialog from '@demicodes/web-ui/files/FileBrowserDialog.vue'
import CloudResetDialog from '@demicodes/web-ui/cloud/CloudResetDialog.vue'
import type { CloudState } from '@demicodes/web-ui/cloud/types'
import GalleryDialogFrame from '../components/GalleryDialogFrame.vue'
import GallerySection from '../components/GallerySection.vue'
import GallerySpecimen from '../components/GallerySpecimen.vue'
import { mockModelMatches, mockVendors, searchMockModels } from '../fixtures/settings'
import { demoInstallCommands, developmentInstallCommands } from '../fixtures/device-installation'
import { createGalleryFileHosts } from '../fixtures/files'
import { useGalleryView } from '../gallery-views'
import { productWould } from '../product-would'
import type { HeadlineText } from '@demicodes/web-ui/ui/ui-text'
import SearchDialog from '@demicodes/web-ui/search/SearchDialog.vue'
import type { SearchRow, SearchSource } from '@demicodes/web-ui/search/search'
import { fixtureSearch, searchConversations } from '../fixtures/search'

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
 * An action the product carries out before the dialog closes: the dialog's
 * button works a moment, as it does for the backend's answer, then it closes.
 */
async function finishSlowly(close: () => void, title: HeadlineText): Promise<void> {
  await slow()
  finish(close, title)
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

/** The search window's conversations, and their rows before the user types, most recently active first. */
const searchable = searchConversations()
const recentRows: SearchRow[] = searchable
  .map(({ messages: _messages, ...row }) => ({ ...row, titleRanges: [], match: null }))
  .toSorted((a, b) => b.lastActiveAt.localeCompare(a.lastActiveAt))
const searchFixtures = fixtureSearch(() => searchable)
/** A search whose first answer fails, as a backend out of reach does; Retry then answers from the fixtures. */
let searchFailures = 0
const failingSearch: SearchSource = (query, signal) => {
  searchFailures += 1
  return searchFailures % 2 === 1
    ? Promise.reject(new Error('The backend could not be reached.'))
    : searchFixtures(query, signal)
}
function openFound(close: () => void, row: SearchRow): void {
  finish(close, row.match ? `Open “${row.title}” at the Matching Message` : `Open “${row.title}”`)
}

/**
 * Hand Off to Your Phone, opened on each of its steps; Open again starts the
 * specimen over, on its step.
 */
const handoffSpecimens = ref(
  (
    [
      ['Confirm · password, or Use Passkey; wrong is refused', 'verify'],
      ['Code · Scan on the Phone stands in for its camera', 'code'],
      ['Expired · Show New Code asks for another', 'expired'],
      ['Handed off · the dialog closes a moment later', 'handedOff'],
    ] as [string, HandoffSpecimenStart][]
  ).map(([variant, start]) => ({ variant, start, ...galleryHandoff(start) })),
)
function restartHandoff(index: number): void {
  const specimen = handoffSpecimens.value[index]!
  handoffSpecimens.value[index] = { ...specimen, ...galleryHandoff(specimen.start) }
}
/** The passkey offer, one specimen for each way the browser can answer Add Passkey. */
const passkeyOffers = [
  ...(
    [
      ['Adds · the browser saves the passkey, and the wizard closes', 'adds'],
      ['Stale sign-in · Add Passkey asks for the password; wrong is refused', 'needsPassword'],
      ['Passkey Ready · the password manager holds a passkey of the account', 'held'],
      ['Fails · the backend could not verify it', 'fails'],
    ] as [string, PasskeyOfferScript][]
  ).map(([variant, script]) => ({ variant, add: galleryPasskeyOffer(script), report: undefined as DeviceReport | undefined })),
  // The sentence under the title names the system's unlock, each as that browser reports itself.
  ...Object.values(DEVICE_REPORTS).map(({ label, report }) => ({
    variant: `${label} · the unlock the sentence names`,
    add: galleryPasskeyOffer('adds'),
    report,
  })),
]
/** Choose Your Password, one specimen for each way it can go on or end. */
const passwordChoices: { variant: string; choose: ReturnType<typeof galleryPasswordChoice>; offer: boolean }[] = (
  [
    ['With the passkey offer · the choice goes on into the offer, in the same dialog', 'saves', true],
    ['Without passkeys · the choice ends the wizard', 'saves', false],
    ['Refused · the backend could not save it, which the step says', 'fails', true],
  ] as [string, PasswordChoiceScript, boolean][]
).map(([variant, script, offer]) => ({ variant, choose: galleryPasswordChoice(script), offer }))
const emailPhases: { variant: string; phase: ChangeEmailPhase }[] = [
  { variant: 'form', phase: { kind: 'form', currentEmail: 'alex@example.com' } },
  { variant: 'form · busy', phase: { kind: 'form', currentEmail: 'alex@example.com', busy: true } },
  { variant: 'verify', phase: { kind: 'verify', email: 'alex@demi.codes' } },
  { variant: 'verify · code resent', phase: { kind: 'verify', email: 'alex@demi.codes', resent: true } },
  { variant: 'done', phase: { kind: 'done', email: 'alex@demi.codes' } },
]
const revokeConfirmations: { variant: string; device: string; home: string; projects: string[] }[] = [
  {
    variant: 'with projects · two of one name',
    device: 'alex-mbp',
    home: '/Users/alex',
    projects: ['/Users/alex/Projects/shop', '/Users/alex/Work/shop', '/Users/alex/Projects/notes'],
  },
  { variant: 'without projects', device: 'build-01', home: '/home/build', projects: [] },
]
/** The name the Rename Device specimen opens on; a rename keeps the new one, as the device's row would. */
const renamedDevice = ref('alex-mbp')
const passwordPhases: { variant: string; phase: ChangePasswordPhase }[] = [
  { variant: 'form', phase: { kind: 'form' } },
  { variant: 'form · busy', phase: { kind: 'form', busy: true } },
  { variant: 'done', phase: { kind: 'done' } },
]
const claudeLink = 'https://claude.com/cai/oauth/authorize?code=true'
const claudePhases: { variant: string; phase: ProviderLoginPhase }[] = [
  { variant: 'Claude Code · contacting', phase: { kind: 'starting', stage: 'preparing' } },
  { variant: 'Claude Code · waiting for the Cloud', phase: { kind: 'starting', stage: 'waiting_for_cloud' } },
  { variant: 'Claude Code · installing', phase: { kind: 'starting', stage: 'installing' } },
  { variant: 'Claude Code · starting', phase: { kind: 'starting', stage: 'starting' } },
  {
    variant: 'Claude Code · link and code',
    phase: { kind: 'code', url: claudeLink, expiresAt: Date.now() + 15 * 60_000, submitted: false },
  },
  {
    variant: 'Claude Code · code refused',
    phase: {
      kind: 'code',
      url: claudeLink,
      expiresAt: Date.now() + 14 * 60_000,
      submitted: false,
      error: 'Invalid code. Please make sure the full code was copied.',
    },
  },
  {
    variant: 'Claude Code · signing in',
    phase: { kind: 'code', url: claudeLink, submitted: true },
  },
  { variant: 'Claude Code · done', phase: { kind: 'done', account: 'alex@example.com · max', active: true } },
  {
    variant: 'Claude Code · failed',
    phase: { kind: 'failed', ...requestFailure('Login failed: the authorization code has expired.') },
  },
  {
    variant: 'Claude Code · failed waiting for the Cloud',
    phase: {
      kind: 'failed',
      ...requestFailure('Your Cloud did not start: its runner did not connect within 60 seconds'),
    },
  },
  {
    variant: 'Claude Code · failed installing',
    phase: {
      kind: 'failed',
      ...requestFailure('Claude Code 2.1.3 could not be installed: the distribution answered 503 Service Unavailable'),
    },
  },
]
const codexPhases: { variant: string; phase: ProviderLoginPhase }[] = [
  { variant: 'Codex · contacting', phase: { kind: 'starting', stage: 'preparing' } },
  {
    variant: 'Codex · device code',
    phase: { kind: 'device', url: 'https://auth.openai.com/codex/device', code: 'HXRV-7K2M', expiresAt: Date.now() + 10 * 60_000 },
  },
  { variant: 'Codex · done · the first account', phase: { kind: 'done', account: 'alex@example.com · Plus', active: true } },
  { variant: 'Codex · done · added beside another', phase: { kind: 'done', account: 'alex@work.example · Pro', active: false } },
]
const grokPhases: { variant: string; phase: ProviderLoginPhase }[] = [
  {
    variant: 'Grok Build · device code',
    phase: { kind: 'device', url: 'https://accounts.x.ai/device', code: 'QK4P-9TWD', expiresAt: Date.now() + 10 * 60_000 },
  },
  { variant: 'Grok Build · done', phase: { kind: 'done', account: 'alex@example.com · SuperGrok', active: true } },
]
const model: SettingsModelDraft = {
  id: 'claude-sonnet-5-5',
  name: 'Claude Sonnet 5.5',
  contextWindow: 200_000,
  outputLimit: 64_000,
  efforts: ['low', 'medium', 'high'],
  extensions: ['.png', '.jpg', '.pdf'],
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
/** A match models.dev lists, as choosing it in Add Model adds it and Edit Model opens it. */
const matchedModel = mockModelMatches[0]!.model
/** The ids of the other models of the specimen's provider, which Edit Model's id may not take. */
const otherIds = ['claude-opus-5-5', 'claude-haiku-5']
const modelEditors: {
  variant: string
  mode: 'edit' | 'view'
  model: SettingsModelDraft
  error?: ShownError
}[] = [
  { variant: 'view', mode: 'view', model },
  { variant: 'edit · Save works a moment', mode: 'edit', model },
  { variant: 'edit · just added from a match', mode: 'edit', model: matchedModel },
  {
    variant: 'edit · just added as Custom, no context window yet',
    mode: 'edit',
    model: { id: 'my-model', name: 'my-model', contextWindow: null, outputLimit: null, efforts: [], extensions: [], fastTier: null },
  },
  { variant: 'edit · an ID another model has', mode: 'edit', model: { ...model, id: 'claude-opus-5-5' } },
  {
    variant: 'edit · refused',
    mode: 'edit',
    model,
    error: correction('Another model of this provider has this ID'),
  },
]
const modelSearches: { variant: string; adding?: boolean; error?: ShownError }[] = [
  { variant: 'search' },
  { variant: 'adding the model chosen', adding: true },
  { variant: 'refused', error: requestFailure('The provider is busy. Try again in a moment.') },
]
const pairingPhases: { variant: string; phase: PairingPhase; installCommands?: DeviceInstallCommands }[] = [
  { variant: 'start the runner', phase: { kind: 'setup' } },
  {
    variant: 'start the runner · development backend over http',
    phase: { kind: 'setup' },
    installCommands: developmentInstallCommands,
  },
  { variant: 'enter the code', phase: { kind: 'code' } },
  { variant: 'pairing', phase: { kind: 'pairing' } },
  { variant: 'connected', phase: { kind: 'done', device: { id: 'demo-device', name: 'alex-mbp' } } },
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
  load: 'loading' | 'ready'
}[] = [
  { variant: 'first time: Cloud', devices, load: 'ready' },
  {
    variant: 'last chose a device',
    devices,
    lastHost: { kind: 'device', deviceId: hosts[0]!.id },
    load: 'ready',
  },
  { variant: 'devices loading', devices: [], load: 'loading' },
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
  error: ShownError | null
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
    // The Cloud's own record of the failure, which no request answered.
    error: { message: 'The system image could not be fetched. Your home files remain saved.', requestId: null },
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
        title="Hand Off to Your Phone"
        note="The phone mark at the sidebar’s foot opens it. The backend decides whether the user confirms it is them first: with the password, or a passkey where the account has one, which holds five minutes. The code is a QR code of the backend’s address with the page the window shows; it lasts ten minutes, and an expired one offers Show New Code. Once the phone opens the page, signed in anew or in the session it had, the dialog says Handed off to its browser and closes by itself."
      >
        <div class="grid items-start gap-6 lg:grid-cols-2">
          <GallerySpecimen v-for="(item, index) in handoffSpecimens" :key="item.variant" wide :variant="item.variant">
            <div class="flex flex-col items-start gap-3">
              <GalleryDialogFrame v-slot="{ open, close }" @reopen="restartHandoff(index)">
                <HandoffDialog
                  :is-open="open"
                  :overlay-store="appOverlayStore"
                  :host="item.host"
                  email="alex@example.com"
                  @close="close"
                />
              </GalleryDialogFrame>
              <Button v-if="item.start === 'code'" size="sm" @click="item.phoneScans()">Scan on the Phone</Button>
            </div>
          </GallerySpecimen>
        </div>
      </GallerySection>
      <GallerySection
        title="Passkey Offer"
        note="A wizard of one step (Responding to the user): on a computer one dialog at the centre over the dimmed page, on a phone the whole screen (Phone page). A host opens it once on a browser, after a sign-in that has just proved the user: a handoff code’s, a password chosen at the first sign-in. Add Passkey adds without the password while the sign-in is fresh; Not Now, Escape and the scrim leave it, and either answer is remembered on that browser, so the wizard does not come back. The sentence under the title names how the device unlocks, from the browser’s system: one specimen for each. A password manager that holds a passkey of the account says so, which is no failure."
      >
        <div class="grid items-start gap-6 lg:grid-cols-2">
          <GallerySpecimen v-for="item in passkeyOffers" :key="item.variant" wide :variant="item.variant">
            <GalleryDialogFrame v-slot="{ open, close }">
              <GalleryDeviceReport :report="item.report">
                <PasskeyOffer
                  :is-open="open"
                  :overlay-store="appOverlayStore"
                  :add="item.add"
                  email="alex@example.com"
                  @answered="productWould('Remember the Answer on This Browser')"
                  @close="close"
                />
              </GalleryDeviceReport>
            </GalleryDialogFrame>
          </GallerySpecimen>
        </div>
      </GallerySection>
      <GallerySection
        title="Choose Your Password"
        note="What an account an administrator made or reset sees at its next sign-in, before anything else of the product opens: a wizard (Responding to the user), on a computer one dialog at the centre over the dimmed page, on a phone the whole screen (Phone page). Its first step asks a new password and its confirmation, without the current one, and has no close button, since it cannot be skipped; Sign Out is the one other way out. Where passkeys are offered, the passkey offer follows in the same dialog and may be left; its answer is remembered on that browser. Type a short or a differing password to see what is said under the fields."
      >
        <div class="grid items-start gap-6 lg:grid-cols-2">
          <GallerySpecimen v-for="item in passwordChoices" :key="item.variant" wide :variant="item.variant">
            <GalleryDialogFrame v-slot="{ open, close }">
              <ChoosePasswordWizard
                :is-open="open"
                :overlay-store="appOverlayStore"
                email="bo@example.com"
                :choose="item.choose"
                :offer="item.offer"
                :add="galleryPasskeyOffer('adds')"
                @answered="productWould('Remember the Answer on This Browser')"
                @done="close"
                @sign-out="productWould('Sign Out and Open the Sign-In Page')"
              />
            </GalleryDialogFrame>
          </GallerySpecimen>
        </div>
      </GallerySection>
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
                email="alex@example.com"
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
                  email="alex@example.com"
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
        note="Removing what the user set up asks first: the title asks, the body says what goes with it, Cancel is the safe answer and the default, the primary button, last and rightmost, which Return presses and the dialog opens on, and the action before it is a danger button, red with no fill, which takes a click, as macOS’s alerts show them."
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
                @confirm="finishSlowly(close, 'Remove the Provider')"
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
                @confirm="finishSlowly(close, 'Remove the Skill Source')"
              >
                <p>Its skills go with it, and the agent is no longer offered them. You can add the repository again later.</p>
              </ConfirmDialog>
            </GalleryDialogFrame>
          </GallerySpecimen>
        </div>
      </GallerySection>
      <GallerySection title="Add Provider" note="A vendor from the catalog, whose endpoint and models are its own, or a compatible API: a proxy or a server speaking one of the protocols Demi implements, which lists the models you add. Choosing either creates the entry at once, with nothing asked first; its page sets the rest.">
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
                @add="(vendor) => finish(close, `Add ${vendor.name} and Show Its Page`)"
                @add-compatible="finish(close, 'Add a Compatible API and Show Its Page')"
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
                @submit-code="productWould(`Give the Code to the ${item.vendor} Sign-In`)"
                @open="(url) => productWould(`Open ${url} in a New Browser Tab`)"
                @retry="productWould(`Start the ${item.vendor} Sign-In Again`)"
              />
            </GalleryDialogFrame>
          </GallerySpecimen>
        </div>
      </GallerySection>
      <GallerySection title="Add Model" note="Add Model is a search and nothing else: typing searches models.dev, and the list under the field offers Custom first, with the typed text, and then the matches with their vendor and context window; the arrows and Return choose, Escape puts the list away. Choosing a row adds the model at once and opens it in Edit Model; a refusal shows under the field.">
        <div class="grid items-start gap-6 xl:grid-cols-2">
          <GallerySpecimen v-for="search in modelSearches" :key="search.variant" wide :variant="search.variant">
            <GalleryDialogFrame v-slot="{ open, close }">
              <AddModelDialog
                :is-open="open"
                :overlay-store="appOverlayStore"
                :search="searchMockModels"
                :adding="search.adding"
                :error="search.error"
                @close="close"
                @add="(added) => finish(close, `Add ${added.name || added.id} and Open It in Edit Model`)"
              />
            </GalleryDialogFrame>
          </GallerySpecimen>
        </div>
      </GallerySection>
      <GallerySection title="Edit Model" note="The one dialog of every model of a provider’s own list, the one just added included: its ID, name, context window, output limit, efforts, Fast tier and files, in that order, every field editable. An ID another model of the provider has is refused under the field and Save waits; a model without a context window cannot be saved until it has one. Save keeps the change, Cancel drops it, Delete Model asks before the model goes. A catalog’s model shows its facts to read.">
        <div class="grid items-start gap-6 xl:grid-cols-2">
          <GallerySpecimen v-for="editor in modelEditors" :key="editor.variant" wide :variant="editor.variant">
            <GalleryDialogFrame v-slot="{ open, close }">
              <ModelDialog
                :is-open="open"
                :overlay-store="appOverlayStore"
                :mode="editor.mode"
                :model="editor.model"
                :error="editor.error"
                :taken-ids="otherIds"
                @close="close"
                @save="(saved) => finishSlowly(close, `Save ${saved.name || saved.id}`)"
                @delete="finish(close, `Ask Before Deleting ${editor.model.name || editor.model.id}`)"
              />
            </GalleryDialogFrame>
          </GallerySpecimen>
        </div>
      </GallerySection>
    </template>

    <template v-if="view === 'devices'">
      <GallerySection title="Rename Device" note="Rename… on a paired device’s row in Settings. The field opens on the current name, selected, so typing replaces it; Return or Rename takes the name trimmed, and an empty one cannot be taken. The specimen keeps the new name and opens on it next time, as the row shows it. A refusal is a toast; the Cloud keeps its name and has no Rename….">
        <div class="grid items-start gap-6 lg:grid-cols-2">
          <GallerySpecimen wide variant="paired device">
            <GalleryDialogFrame v-slot="{ open, close }">
              <DeviceRenameDialog
                :is-open="open"
                :overlay-store="appOverlayStore"
                :name="renamedDevice"
                :max-length="64"
                @close="close"
                @rename="renamedDevice = $event"
              />
            </GalleryDialogFrame>
          </GallerySpecimen>
        </div>
      </GallerySection>
      <GallerySection title="Add Device" note="One flow for a local computer, a remote server or a headless machine: start the runner, paste its pairing code, use the connected device.">
        <div class="grid items-start gap-6 lg:grid-cols-2">
          <GallerySpecimen v-for="(item, index) in pairingPhases" :key="item.variant" wide :variant="item.variant">
            <GalleryDialogFrame v-slot="{ open, close }" @reopen="pairingShown[index] = item.phase">
              <DevicePairingDialog
                :is-open="open"
                :overlay-store="appOverlayStore"
                :install-commands="item.installCommands ?? demoInstallCommands"
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
      <GallerySection title="Revoke Device" note="Asked before a device is revoked. The projects on it go with it, each named by its directory with its location beside it, so two of one name differ; their files and conversations stay. A device without projects is asked about plainly.">
        <div class="grid items-start gap-6 lg:grid-cols-2">
          <GallerySpecimen v-for="item in revokeConfirmations" :key="item.variant" wide :variant="item.variant">
            <GalleryDialogFrame v-slot="{ open, close }">
              <DeviceRevokeDialog
                :is-open="open"
                :overlay-store="appOverlayStore"
                :device="item.device"
                :projects="item.projects"
                :home="item.home"
                @close="close"
                @revoke="finishSlowly(close, `Revoke the Device ${item.device}`)"
              />
            </GalleryDialogFrame>
          </GallerySpecimen>
        </div>
      </GallerySection>
    </template>

    <template v-if="view === 'workspace'">
      <GallerySection
        title="Delete Conversations"
        note="Delete in a conversation’s sidebar menu, a selection’s menu or an archived row asks first: the title names the one conversation or counts several, the body says that their messages and files go and that this cannot be undone, and that the files they changed in their projects stay. Delete is a danger button, red with no fill, and Cancel after it, last, the safe answer and the default."
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
                @delete="finishSlowly(close, item.titles.length === 1 ? 'Delete the Conversation' : `Delete ${countOf(item.titles.length, 'Conversation')}`)"
              />
            </GalleryDialogFrame>
          </GallerySpecimen>
        </div>
      </GallerySection>
      <GallerySection title="New Project" note="A project on the Cloud or a device. The first time it opens on the Cloud; after that on the kind and the device chosen last, which each specimen remembers across Close and Open as the product’s preference does. Moving a conversation between existing projects is the header’s menus, not a dialog. The directory completes from the device’s folders as it is typed: the folder before the caret lists under the field, filtered fuzzily by what follows its last slash; the menu opens with its first row selected, and again whenever its rows change; ↓ and ↑ and the moving pointer move the selection, which is the one lit row, Tab completes it and the menu goes on into it, and Return creates the project with what the field holds; Escape or a click anywhere outside it puts the menu away and leaves the focus where the click put it, and with no menu Tab moves the focus as anywhere. The button says Add Project for a directory that exists and Create Project for one Demi will make, as the device’s folders tell; Projects/demi exists, Projects/weather-app does not.">
        <div class="grid items-start gap-6 lg:grid-cols-2">
          <GallerySpecimen v-for="(form, index) in projectForms" :key="form.variant" wide :variant="form.variant">
            <GalleryDialogFrame v-slot="{ open, close }">
              <WorkspaceDialog
                :is-open="open"
                :overlay-store="appOverlayStore"
                :devices="form.devices"
                :last-host="projectLastHosts[index]"
                :load="form.load"
                :source-for="sourceFor"
                :places-for="placesFor"
                @close="close"
                @create="(draft) => finishSlowly(close, draft.kind === 'cloud' ? `Create the Cloud Project ${draft.name} and Open a New Conversation in It` : `Set Up the Project at ${draft.path} and Open a New Conversation in It`)"
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

    <template v-if="view === 'search'">
      <GallerySection
        title="Search"
        note="⌘K or Search in the sidebar opens it. The recent conversations until the user types; then, once typing pauses, title matches first and then message matches, each most recently active first, the matches marked in the title and the line. The window fits its list up to a limit and keeps its field in place. ↑ and ↓ move, Return or a click opens, Escape closes. Try TS2307, login or 路径."
      >
        <div class="grid items-start gap-6 xl:grid-cols-2">
          <GallerySpecimen wide variant="live">
            <GalleryDialogFrame v-slot="{ open, close }">
              <SearchDialog
                :is-open="open"
                :overlay-store="appOverlayStore"
                :recent="recentRows"
                :search="searchFixtures"
                @close="close"
                @open="(row) => openFound(close, row)"
              />
            </GalleryDialogFrame>
          </GallerySpecimen>
          <GallerySpecimen wide variant="search fails, then Retry answers">
            <GalleryDialogFrame v-slot="{ open, close }">
              <SearchDialog
                :is-open="open"
                :overlay-store="appOverlayStore"
                :recent="recentRows"
                :search="failingSearch"
                @close="close"
                @open="(row) => openFound(close, row)"
              />
            </GalleryDialogFrame>
          </GallerySpecimen>
          <GallerySpecimen wide variant="no conversations yet">
            <GalleryDialogFrame v-slot="{ open, close }">
              <SearchDialog
                :is-open="open"
                :overlay-store="appOverlayStore"
                :recent="[]"
                :search="fixtureSearch(() => [])"
                @close="close"
                @open="(row) => openFound(close, row)"
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
