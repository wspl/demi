<script setup lang="ts">
import { computed, reactive, ref } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsDialog from '@demicodes/web-ui/settings/SettingsDialog.vue'
import SettingsAccount from '@demicodes/web-ui/settings/SettingsAccount.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import type { SettingsTab } from '@demicodes/web-ui/settings/types'
import GalleryOverlayWell from '../components/GalleryOverlayWell.vue'
import GallerySection from '../components/GallerySection.vue'
import GalleryAbout from '../components/GalleryAbout.vue'
import type { NoticesAnswer } from '../fixtures/acknowledgements'
import GallerySettingsFull from '../components/GallerySettingsFull.vue'
import GallerySettingsRows from '../components/GallerySettingsRows.vue'
import GallerySkillsShowcase from '../components/GallerySkillsShowcase.vue'
import GallerySkillsCalls from '../components/GallerySkillsCalls.vue'
import GalleryWebSearchCalls from '../components/GalleryWebSearchCalls.vue'
import GalleryWebSearchShowcase from '../components/GalleryWebSearchShowcase.vue'
import GallerySpecimen from '../components/GallerySpecimen.vue'
import { settingsSections as productSections, withUsersSection } from '@demicodes/web-ui/settings/sections'
import GallerySettingsProviders from '../components/GallerySettingsProviders.vue'
import { createSettingsState, gallerySessions, incompleteProviders, mockProviders } from '../fixtures/settings'
import { galleryPageHost, openExaSignIn, skillsPlugin, webPlugin } from '../fixtures/plugins'
import { slow } from '../fixtures/slow'
import { PLUGIN_PAGES } from '../generated/pages'
import { providePageHost, withPluginSections } from '@demicodes/web-ui/plugins/page'
import { useGalleryView } from '../gallery-views'
import GalleryCredentials from '../components/GalleryCredentials.vue'
import GalleryUsers from '../components/GalleryUsers.vue'
import { createUsersState } from '../fixtures/users'
import GallerySubagents from '../components/GallerySubagents.vue'
import CredentialDialog from '@demicodes/web-ui/settings/CredentialDialog.vue'
import type { SettingsCredentialDraft } from '@demicodes/web-ui/settings/types'
import { createCredentialState, demoCredentials } from '../fixtures/credentials'
import SubagentProfileDialog from '@demicodes/web-ui/settings/SubagentProfileDialog.vue'
import type { SettingsSubagentDraft } from '@demicodes/web-ui/settings/types'
import { createSubagentState, demoSubagentProfiles } from '../fixtures/subagent-profiles'
import { productWould } from '../product-would'
import { t as webSearchText } from '@demicodes/plugin-web/locales/text'
import { demoProviders } from '../fixtures/catalog'
import { composerModels } from '../fixtures/context-limits'
import SettingsDevices from '@demicodes/web-ui/settings/SettingsDevices.vue'
import Segmented from '@demicodes/web-ui/ui/Segmented.vue'
import type { SettingsSessionHost } from '@demicodes/web-ui/settings/types'
import { demoInstallCommands, demoDeviceReport } from '../fixtures/device-installation'
import { DEMO_RUNNER_RELEASE, DIRECT_SCENARIOS, galleryDevices, useGalleryDevices, type DirectScenario } from '../fixtures/devices'
import type { CloudState } from '@demicodes/web-ui/cloud/types'

const { view } = useGalleryView()

/**
 * The Devices specimen's devices, its page open on alex-mbp, whose path the
 * segmented control sets to each reason its page gives.
 */
/**
 * How often a full mock's open section was chosen again on its rail, which
 * mounts the section anew, so a page it opened inside itself gives way to
 * the section's own, as the product's address does.
 */
const fullReselects = ref(0)
const directDevices = useGalleryDevices(() => galleryDevices())
directDevices.shown.value = 'mac'
const directScenario = computed({
  get: () => directDevices.scenarios.value['mac'] ?? 'connected',
  set: (scenario: DirectScenario) => directDevices.setScenario('mac', scenario),
})
/** The Cloud beside them, whose page says it is always connected via the relay. */
const directCloud: CloudState = {
  state: 'running',
  operationId: null,
  phase: null,
  error: null,
  volumes: { systemBytes: 3 * 1024 ** 3, homeBytes: 9 * 1024 ** 3 },
  limits: { systemBytes: 16 * 1024 ** 3, homeBytes: 64 * 1024 ** 3 },
  newerImage: false,
  deviceId: 'cloud',
  report: demoDeviceReport('linux', DEMO_RUNNER_RELEASE),
}

const anatomy: [string, string][] = [
  [
    'Shell',
    'One large dialog. The rail sits on the page surface with the account name on top and a filter under it, the page on the dialog surface, so it reads like the app itself. A whole unused page stays on the rail and is disabled with an In development tooltip: Data & privacy. A plugin’s section is on the rail while its plugin is on. On a narrow screen, where the app’s side panes become overlays, the dialog fills the window: square corners, no scrim around it. In the phone layout (below 768 px, however wide the dialog still is) it reads as iOS Settings and never shows the rail beside a page: the rail is a list of the sections with rows a finger can hit, and a section slides over it as a page of its own, with a back button to the list. The Phone page shows it on a real phone frame. The filter finds single settings by their label or what people call them (dark mode finds Theme), lists them under their section, and opens one with its row highlighted; Escape in the filter clears it before it closes the dialog.'
  ],
  [
    'Page',
    'A title, one line under it, then titled groups. A group is a card of rows: label and explanation left, the control right.'
  ],
  [
    'Dialogs',
    'A dialog that opens from a page keeps its title, search and buttons in place; only the list or form between them scrolls.'
  ],
  [
    'Readouts',
    'A value the control produces (a size, a temperature) reads out beside the control, never in the explanation under the label.'
  ],
  [
    'Writes',
    'Theme, tone, accent and text size apply as they change. There is no saving indicator. Change email, change password, provider login and Cloud reset are explicit operations with their own phases, not a settings save.'
  ],
  [
    'Cloud',
    'Cloud shows its shared-environment description and its storage: each filesystem’s current size and the most it may grow to, or only the most before its first start. There is no connection or lifecycle status. Reset progress and errors belong to the reset dialog.'
  ],
  [
    'Providers',
    'A bare rail of providers beside the selected one, with no surface of its own. Every supported subscription is always listed. A dot means attention, for every kind of provider alike: none while it works, yellow while it is not set up, red only when it failed or cannot be reached. A vendor entry shows its vendor’s endpoint as plain text and lists the vendor’s models, which a manual list can replace; a compatible API sets its protocol, its endpoint and an optional key at any time, lists only the models you add and has nothing to refresh. A subscription entry manages accounts. Adding creates the entry at once, selected, on the page every entry has; Add Model adds the model chosen at once and opens it in Edit Model, the dialog every model opens.'
  ],
  [
    'Credentials',
    'Changing the email asks for the new address and the current password, then a code sent to the new address. Changing the password asks for the current one and the new one twice; length and the match are checked in the dialog, the current password by the server.'
  ],
  [
    'Subagents',
    'A switch that turns subagents on or off for the user; Maximum subagents and Maximum depth, each a number or Unlimited, which asks first in a dialog whose default button, Cancel, keeps the number; then the user’s profiles, each with its own switch. A row names the profile’s model, its prompt and whether its children spawn; a profile whose provider entry, model, effort or tier is gone carries Unavailable, with the missing part as its tooltip. The editor chooses the parent’s model or any provider’s with its effort and tier, and the parent’s prompt or text that replaces it.'
  ],
  [
    'Agent credentials',
    'Keys, tokens and passwords the user keeps for their agents, listed by name with each field’s label and a tag for its kind, Kept until and Last used. A secret field shows dots and an eye that reads its value from the backend and shows it to the user; the value goes again when the pointer or the focus leaves the row. New Credential and Edit open the editor: name, note, how long to keep it, and one card per field with its label, kind and value; a saved field’s value left empty keeps it. Delete asks first.'
  ],
  [
    'Skills',
    'A plugin’s section: the rail lists it under Agent, and the sidebar has an entry that opens it, both only while the Skills plugin is on. Git sources are packs of SKILL.md files; a source’s state is one status label after its name, and a skill’s labels say why it is off or never offered. The showcase below pins every state at once.'
  ],
  [
    'Web Search',
    'A plugin’s section, under Agent in the settings dialog only, while the Web plugin is on; the app’s sidebar has no entry for it, as for Providers. It holds the user’s Exa account, signed in or not, the API key, typed and checked or saved and shown by its last four characters, and which of them searches use. The specimens below pin each state and run the calls.'
  ],
  [
    'Users',
    'Under Workspace, only for the master and admins. One row per account: nickname with You on the user’s own, Disabled on a disabled one and Hasn’t Chosen a Password on one still to choose its own (a new account, or one an administrator reset), then email, role and when it was last active. A row’s ··· menu has only what the user may do to that account: an admin has Reset Password…, Sign-In Code…, Sign Out Everywhere… and Disable Account… on users and nothing on a peer, the master or their own row; the master also has Change Role… and nothing on their own. A disabled account has Enable Account, which asks nothing, in place of the last three. Sign-In Code… shows the Hand Off to Your Phone dialog’s QR code for the account, after the administrator’s own confirmation. Add User… asks the email and the role; the server makes the first password, which the last step shows with Copy and New Password beside the sign-in code. Reset Password… asks first and ends on the same last step. Add User…, Reset Password… and Sign-In Code… ask the administrator’s own confirmation when the session was not verified in the last five minutes.'
  ],
  [
    'Data',
    'The rail entry is disabled with an In development tooltip because every control on the page is deferred. The Data & Privacy specimen below shows its page.'
  ],
  [
    'Sign-in',
    'Each subscription signs in the way its vendor does. Claude Code prints a token from its own CLI, so the dialog walks through install, command and paste. Codex and Grok Build confirm a device code in the browser while the dialog waits.'
  ],
]

const account = { id: 'alex', name: 'Alex' }
const full = createSettingsState()
/** Providers where the only API-key entry is complete: Add, then a vendor, adds an incomplete one. */
const adding = createSettingsState()
adding.providers = mockProviders().filter((entry) => ['claude-code', 'anthropic'].includes(entry.id))
adding.selectedProviderId = 'anthropic'
adding.providerDetailOpen = true
/** Providers each lacking a part: a sign-in, a key, a model, a key and a model. */
const incomplete = createSettingsState()
incomplete.providers = incompleteProviders()
incomplete.selectedProviderId = 'google'
incomplete.providerDetailOpen = true
/** What the Notifications specimen's stand-in browser answers when asked. */
const browserAnswers = [
  { value: 'granted', label: 'Browser Allows' },
  { value: 'denied', label: 'Browser Blocks' },
] as const
/** A new answer is a browser whose site settings were reset: it asks again. */
function answerAs(answer: 'granted' | 'denied'): void {
  full.notifications.answer = answer
  full.notifications.permission = 'default'
  full.notifications.enabled = false
}
/** What the About specimen's stand-in server answers for the notices. */
const noticesAnswers = [
  { value: 'notices', label: 'Notices' },
  { value: 'none', label: 'Development Build' },
  { value: 'failure', label: 'Failure' },
] as const
const noticesAnswer = ref<NoticesAnswer>('notices')
/** The account specimen on a server without mail. */
const noMail = ref({ name: 'Alex' })
/** The specimen's signed-in browsers: Sign Out drops one, as the product's list would. */
const noMailSessions = reactive<SettingsSessionHost>({
  sessions: gallerySessions(),
  signOut: async (id: string) => {
    noMailSessions.sessions = noMailSessions.sessions.filter((session) => session.id !== id)
    productWould('Sign the Browser Out')
  },
})
// The plugins' sections reach the fixture's skills plugin, and show while the
// fixture's Plugins page has their plugin on, as the product's do.
providePageHost(
  galleryPageHost(
    { skills: skillsPlugin(full.skills), web: webPlugin(full.web) },
    { openLink: openExaSignIn },
  ),
)
// The full mock's user is the master, whose rail lists Users.
const sections = computed(() =>
  withUsersSection(
    withPluginSections(productSections(), PLUGIN_PAGES, (plugin) =>
      full.plugins.some((entry) => entry.id === plugin && entry.enabled),
    ),
    'master',
  ),
)
/** The Users specimens: the master's page, an admin's, and the master's at phone width. */
const usersAsMaster = createUsersState(('alex'))
const usersAsAdmin = createUsersState('ana')
const usersNarrow = createUsersState(('alex'))
const fullTab = ref<SettingsTab | null>('models')
const fullNarrowTab = ref<SettingsTab | null>(null)
// Each pinned dialog closes from its own Close and opens again in place from Open,
// as the product's settings dialog does.
const fullOpen = ref(true)
const fullNarrowOpen = ref(true)

// The Subagents section with subagents turned off: the profiles stay, muted.
const subagentsOff = createSubagentState(false)

/** A profile pinned in its editor, which closes from its buttons and opens again from Open. */
function pinnedEditor(id: string | null) {
  const profile = demoSubagentProfiles().find((candidate) => candidate.id === id)
  const draft: SettingsSubagentDraft = profile
    ? {
        name: profile.name,
        description: profile.description,
        model: profile.model,
        contextLimit: profile.contextLimit,
        instructions: profile.instructions,
        canSpawn: profile.canSpawn,
      }
    : { name: '', description: '', model: null, contextLimit: null, instructions: null, canSpawn: true }
  return { open: ref(true), draft }
}
/** The Credentials section without a credential: its list says how one comes. */
const noCredentials = createCredentialState([])

/** A credential pinned in its editor, which closes from its buttons and opens again from Open. */
function pinnedCredentialEditor(id: string | null) {
  return { open: ref(true), credential: demoCredentials().find((candidate) => candidate.id === id) ?? null }
}
const newCredentialEditor = pinnedCredentialEditor(null)
const editedCredentialEditor = pinnedCredentialEditor('cred-linear')

async function savedCredential(editor: ReturnType<typeof pinnedCredentialEditor>, draft: SettingsCredentialDraft) {
  await slow()
  editor.open.value = false
  productWould(`The product would save the credential ${draft.name}`)
}

function deletedCredential(editor: ReturnType<typeof pinnedCredentialEditor>) {
  editor.open.value = false
  productWould(`The product would delete the credential ${editor.credential?.name}`)
}

const inheritEditor = pinnedEditor(null)
const ownEditor = pinnedEditor('explore')
const contextEditor = pinnedEditor('scout')

async function saved(editor: ReturnType<typeof pinnedEditor>, draft: SettingsSubagentDraft) {
  await slow()
  editor.open.value = false
  productWould(`The product would save the profile ${draft.name}`)
}

function deleted(editor: ReturnType<typeof pinnedEditor>) {
  editor.open.value = false
  productWould(`The product would delete the profile ${editor.draft.name}`)
}

</script>

<template>
  <div class="flex flex-col gap-10">
    <template v-if="view === 'settings'">
      <GallerySection
        title="Settings"
        note="The product settings dialog and each of its panels, pinned open with mocked state."
      >
        <dl
          class="grid max-w-3xl grid-cols-[7rem_minmax(0,1fr)] gap-x-4 gap-y-2 text-[13px] leading-5"
        >
          <template v-for="[term, detail] in anatomy" :key="term">
            <dt class="select-none text-fg-subtle">{{ term }}</dt>
            <dd class="text-fg-muted">{{ detail }}</dd>
          </template>
        </dl>
      </GallerySection>

      <GallerySection
        title="Rows"
        note="Label and explanation on the left, the control on the right. Each row decides by its own content: where the label would be left less than 9rem beside its control, the control drops under it, as in iOS Settings, whatever the language or the card’s width. A text field, a slider, a segmented control or a menu button then fills the row, the segments sharing the width with each label whole and the menu button keeping its value at the start and its chevron at the end, while a switch and a button keep their size at the right edge. A title wraps at its words and is cut only where one word is wider than the line. What needs the row’s whole width, such as an account’s usage bars, takes a line of its own under the name and the controls at every width, the controls then aligned with the name; Refresh reads the usage again."
      >
        <GallerySpecimen variant="Wide" wide>
          <div class="w-full max-w-xl">
            <GallerySettingsRows />
          </div>
        </GallerySpecimen>
        <GallerySpecimen variant="Narrow · 320px">
          <div class="w-[320px] max-w-full">
            <GallerySettingsRows />
          </div>
        </GallerySpecimen>
        <GallerySpecimen variant="Phone · 390px · Japanese">
          <div class="w-[390px] max-w-full" lang="ja">
            <GallerySettingsRows ja />
          </div>
        </GallerySpecimen>
        <GallerySpecimen variant="Narrow · 280px · Japanese">
          <div class="w-[280px] max-w-full" lang="ja">
            <GallerySettingsRows ja />
          </div>
        </GallerySpecimen>
      </GallerySection>

      <GallerySection
        title="Devices · Direct Channel"
        note="A device’s row names it and says its system and its state; the reason waits on the device’s own page, which the row opens and whose back button returns to the list. The page’s header names the device and its state, Connected via This Computer, LAN or P2P by the kind of path the peer stands on, or Connected via relay, its dot the state’s colour, green while online whichever path; its icon is as tall as the name and status lines together. Connection is one group of rows, as macOS’s Wi-Fi settings: Route, Automatic, Prefer P2P or Relay Only, with one line for the chosen one; P2P, while the device is online and not Relay Only, its subtitle why it is not connected, in the design table’s words, or the kind and the address it is connected through, LAN · 192.168.1.20, with Try Again, there without a P2P connection and loading only for the attempt it started, and Details…, which opens Connection Details; and Latency, both paths’ figures from the last measurement, the peer’s path by its kind, Measuring… before the first, with Measure, which measures again and is loading only for the measurement it started; the page measures by itself each time it shows, and the figures stay until the new ones replace them. Rename… renames; Revoke… asks first, and the device leaves the list. build-01 is offline: its header gives the command that starts it, with Copy. Set alex-mbp’s path to see each state, LAN, This Computer and P2P being the three kinds of P2P path; the Cloud’s page says only that it is connected via relay."
      >
        <div class="flex w-full max-w-2xl flex-col gap-4">
          <Segmented v-model="directScenario" :options="DIRECT_SCENARIOS" size="sm" />
          <div class="@container rounded-xl border border-line bg-surface-dialog p-6">
            <SettingsDevices
              :cloud="directCloud"
              :devices="directDevices.devices.value"
              :shown="directDevices.shown.value"
              :runner-release="DEMO_RUNNER_RELEASE"
              :overlay-store="appOverlayStore"
              :install-commands="demoInstallCommands"
              :claim-device="directDevices.claim"
              :name-max-length="64"
              @show="directDevices.shown.value = $event"
              @set-route="directDevices.setRoute"
              @try-now="directDevices.tryNow"
              @measure="directDevices.measure"
              @revoke="directDevices.revoke"
              @rename="directDevices.rename"
              @reset-cloud="productWould('Reset the Cloud Environment')"
              @retry="productWould('Load the Devices Again')"
            />
          </div>
        </div>
      </GallerySection>

      <GallerySection
        title="Users"
        note="Settings › Users as the master sees it, as an admin sees it, and at a phone’s width. Every control acts on its own specimen: Add User… adds a row (alex@example.com is taken, and the new row comes with its password and QR code in its last step; Lena and Joe show the new-account and disabled looks), Change Role… moves a user to admin, Reset Password… marks the account as still to choose its password, the first of those needing a confirmation asks for a password (any but wrong confirms), and Disable Account… and Sign Out Everywhere… toast what the product would do to the account’s sessions. As the admin, Ana, there is no Change Role…, and the master’s row, the other admin’s and her own have no menu."
      >
        <GallerySpecimen variant="Master" wide>
          <div class="@container w-full max-w-2xl rounded-xl border border-line bg-surface-dialog p-6">
            <GalleryUsers :state="usersAsMaster" />
          </div>
        </GallerySpecimen>
        <GallerySpecimen variant="Admin" wide>
          <div class="@container w-full max-w-2xl rounded-xl border border-line bg-surface-dialog p-6">
            <GalleryUsers :state="usersAsAdmin" />
          </div>
        </GallerySpecimen>
        <GallerySpecimen variant="Narrow · 360px">
          <div class="@container w-[360px] max-w-full rounded-xl border border-line bg-surface-dialog p-4">
            <GalleryUsers :state="usersNarrow" />
          </div>
        </GallerySpecimen>
      </GallerySection>

      <GallerySection
        title="Providers · Adding"
        note="Add in the API Keys caption offers a compatible API or a vendor, and choosing one creates the entry at once, selected, with the yellow dot and nothing red. Pick Google, paste any key into the API key field, and Add Model finds Gemini on models.dev, adds it and opens it in Edit Model. Or add a compatible API: it is Compatible API, then Compatible API (2), speaking Anthropic Messages with no endpoint; set its protocol, its endpoint and, if you like, a key, and add a model. The dot goes with the last part."
      >
        <div class="h-[640px] w-full max-w-4xl @container rounded-xl border border-line bg-surface-dialog p-6">
          <GallerySettingsProviders :state="adding" />
        </div>
      </GallerySection>

      <GallerySection
        title="Providers · Incomplete"
        note="An entry that cannot run yet wears the yellow dot and nothing red, since nothing has failed: a compatible API with no model says No models yet in plain text, with Add Model and no refresh, and a subscription nobody signed in to says so. Google, a vendor, shows its endpoint as plain text and lacks its key; Lab Server lacks its first model, Compatible API, just added, its endpoint and a model, with its key optional, Codex its sign-in. The dot goes the moment the last part is saved."
      >
        <div class="h-[640px] w-full max-w-4xl @container rounded-xl border border-line bg-surface-dialog p-6">
          <GallerySettingsProviders :state="incomplete" />
        </div>
      </GallerySection>

      <GallerySection
        title="Notifications"
        note="Browser notifications are off until turned on, and belong to this browser. Turning them on asks the browser; switch what the stand-in browser answers to see each. Blocked, the switch goes back off and the row says where to allow them again; a browser that already blocks the site answers without asking. The three switches under it choose what notifies, and wait for notifications to be on."
      >
        <div class="flex w-full max-w-2xl flex-col gap-4">
          <Segmented
            :model-value="full.notifications.answer"
            :options="browserAnswers"
            size="sm"
            @update:model-value="answerAs"
          />
          <div class="rounded-xl border border-line bg-surface-dialog p-6">
            <GallerySettingsFull tab="notifications" :state="full" />
          </div>
        </div>
      </GallerySection>

      <GallerySection
        title="About"
        note="The version, and Acknowledgements, a page of its own with a back button to About: the third-party notices of the release, read when the page opens. Switch what the stand-in server answers: the notices; none, as a development build has; or a failure, whose Retry reads them again."
      >
        <div class="flex w-full max-w-2xl flex-col gap-4">
          <Segmented
            v-model="noticesAnswer"
            :options="noticesAnswers"
            size="sm"
          />
          <div class="@container rounded-xl border border-line bg-surface-dialog p-6">
            <GalleryAbout :answer="noticesAnswer" />
          </div>
        </div>
      </GallerySection>

      <GallerySection
        title="Data & Privacy"
        note="The page behind the rail’s disabled Data & Privacy entry, whose controls are all deferred: diagnostics and delete-all. Exporting conversations and share links have no entry."
      >
        <GallerySpecimen variant="Page" wide>
          <div class="w-full max-w-2xl">
            <GallerySettingsFull tab="data" :state="full" />
          </div>
        </GallerySpecimen>
      </GallerySection>

      <GallerySection
        title="Full Agent Settings"
        note="A stress test: expiring auth, an unreachable local model, a quota nearly spent, disabled entries, nested rows, long paths. Visible accounts reuse usage requests for one minute. Automatic refresh keeps existing meters and buttons still and enabled. Only a manual refresh spins its button; it bypasses the TTL or joins an automatic request already running."
      >
        <GalleryOverlayWell size="tall">
          <Button v-if="!fullOpen" size="md" @click="fullOpen = true">Open</Button>
          <SettingsDialog
            v-slot="{ section }"
            v-model:tab="fullTab"
            @reselect="fullReselects += 1"
            :is-open="fullOpen"
            :overlay-store="appOverlayStore"
            :account="account"
            :sections="sections"
            @close="fullOpen = false"
          >
            <GallerySettingsFull :key="fullReselects" :tab="section ?? ''" :state="full" />
          </SettingsDialog>
        </GalleryOverlayWell>
      </GallerySection>

      <GallerySection
        title="Account · Server Without Mail"
        note="A server with no mail sender cannot send the code an email change needs: the Email row says so from the start and its Change is off, its tooltip saying why. Clearing the display name says under the field what it needs and keeps the empty field; Escape gives the field its name back."
      >
        <div class="max-w-3xl rounded-xl border border-line bg-surface-dialog p-6">
          <SettingsAccount
            v-model:name="noMail.name"
            :overlay-store="appOverlayStore"
            account-id="alex"
            email="alex@example.com"
            email-unavailable
            :email-phase="{ kind: 'form', currentEmail: 'alex@example.com' }"
            :password-phase="{ kind: 'form' }"
            :sessions="noMailSessions"
            @change-password="productWould('Open Change Password')"
            @submit-email="productWould('Send the Code')"
            @verify-email="productWould('Check the Code')"
            @resend-email="productWould('Send Another Code')"
            @submit-password="productWould('Change the Password')"
            @sign-out="productWould('Sign Out')"
            @delete-account="productWould('Delete the Account')"
          />
        </div>
      </GallerySection>

      <GallerySection
        title="Subagents · Off"
        note="Subagents turned off: no agent spawns, the profiles stay and can still be edited. The disabled archivist’s model is gone too, and the scout’s effort is no longer offered."
      >
        <div class="max-w-3xl rounded-xl border border-line bg-surface p-6">
          <GallerySubagents :state="subagentsOff" />
        </div>
      </GallerySection>

      <GallerySection
        title="Credentials"
        note="The user’s credentials, each with its fields and a tag for their kind, Kept until and Last used. The eye of a secret field reads its value and shows it, and the value goes when the pointer or the focus leaves the row; the Linear sign-in’s user name is visible, its password hidden. Click a row or Edit to change it; New Credential adds one. Every write waits a beat, and a name another credential has is refused."
      >
        <div class="max-w-3xl rounded-xl border border-line bg-surface p-6">
          <GalleryCredentials :state="full.credentials" />
        </div>
      </GallerySection>

      <GallerySection
        title="Credentials · Empty"
        note="No credential yet: the list says how one comes. New Credential adds one to this list."
      >
        <div class="max-w-3xl rounded-xl border border-line bg-surface p-6">
          <GalleryCredentials :state="noCredentials" />
        </div>
      </GallerySection>

      <GallerySection
        title="Credential Editor · New"
        note="A new credential: one empty hidden field, kept always. A field’s name is made from its label and shown under it. Create Credential stays off until the name, the labels and the values are filled; a problem shows under its field once the field is left. Add Field adds a card, up to ten; a card’s bin removes it, except the last."
      >
        <GalleryOverlayWell size="tall">
          <Button v-if="!newCredentialEditor.open.value" size="md" @click="newCredentialEditor.open.value = true">Open</Button>
          <CredentialDialog
            :is-open="newCredentialEditor.open.value"
            :overlay-store="appOverlayStore"
            :credential="newCredentialEditor.credential"
            @close="newCredentialEditor.open.value = false"
            @save="savedCredential(newCredentialEditor, $event)"
          />
        </GalleryOverlayWell>
      </GallerySection>

      <GallerySection
        title="Credential Editor · Edit"
        note="The Linear sign-in in its editor: its fields keep their names, and a value left empty keeps the saved one. Keep until shows the time it has now; choosing one keeps it that long from now."
      >
        <GalleryOverlayWell size="tall">
          <Button v-if="!editedCredentialEditor.open.value" size="md" @click="editedCredentialEditor.open.value = true">Open</Button>
          <CredentialDialog
            :is-open="editedCredentialEditor.open.value"
            :overlay-store="appOverlayStore"
            :credential="editedCredentialEditor.credential"
            @close="editedCredentialEditor.open.value = false"
            @save="savedCredential(editedCredentialEditor, $event)"
            @delete="deletedCredential(editedCredentialEditor)"
          />
        </GalleryOverlayWell>
      </GallerySection>

      <GallerySection
        title="Profile Editor · Inherits"
        note="A new profile: the parent’s model and no instructions of its own, its children may spawn. Create Profile stays off until the name and the description are filled. The model menu is the composer’s, with Parent’s Model at its top."
      >
        <GalleryOverlayWell size="tall">
          <Button v-if="!inheritEditor.open.value" size="md" @click="inheritEditor.open.value = true">Open</Button>
          <SubagentProfileDialog
            :is-open="inheritEditor.open.value"
            :overlay-store="appOverlayStore"
            mode="create"
            :profile="inheritEditor.draft"
            :providers="demoProviders"
            :models="composerModels"
            :catalog-models="composerModels"
            @close="inheritEditor.open.value = false"
            @save="saved(inheritEditor, $event)"
          />
        </GalleryOverlayWell>
      </GallerySection>

      <GallerySection
        title="Profile Editor · Own Instructions, Another Provider’s Model"
        note="The explore profile: GPT-5 of OpenAI with Fast, while the conversations run on Anthropic, instructions added to Demi’s own, and no spawning."
      >
        <GalleryOverlayWell size="tall">
          <Button v-if="!ownEditor.open.value" size="md" @click="ownEditor.open.value = true">Open</Button>
          <SubagentProfileDialog
            :is-open="ownEditor.open.value"
            :overlay-store="appOverlayStore"
            mode="edit"
            :profile="ownEditor.draft"
            :providers="demoProviders"
            :models="composerModels"
            :catalog-models="composerModels"
            @close="ownEditor.open.value = false"
            @save="saved(ownEditor, $event)"
            @delete="deleted(ownEditor)"
          />
        </GalleryOverlayWell>
      </GallerySection>

      <GallerySection
        title="Profile Editor · Own Context"
        note="The scout profile on Claude Opus, whose 1M window offers a choice: its subagents use 500K while the user’s conversations on Opus keep their own limit. Default names the window that limit gives; the Context row leaves with Parent’s Model or a model of 500K or less."
      >
        <GalleryOverlayWell size="tall">
          <Button v-if="!contextEditor.open.value" size="md" @click="contextEditor.open.value = true">Open</Button>
          <SubagentProfileDialog
            :is-open="contextEditor.open.value"
            :overlay-store="appOverlayStore"
            mode="edit"
            :profile="contextEditor.draft"
            :providers="demoProviders"
            :models="composerModels"
            :catalog-models="composerModels"
            @close="contextEditor.open.value = false"
            @save="saved(contextEditor, $event)"
            @delete="deleted(contextEditor)"
          />
        </GalleryOverlayWell>
      </GallerySection>

      <GallerySection
        title="Full · Narrow"
        note="A phone-width window: the dialog fills it edge to edge with square corners and no scrim around it, the close button stays in its corner. It opens on the list of sections; a section slides over it as its own page with Settings to go back, and slides away again, and Providers opens on its list of providers, not on one of them."
      >
        <GalleryOverlayWell size="narrow">
          <Button v-if="!fullNarrowOpen" size="md" @click="fullNarrowOpen = true">Open</Button>
          <SettingsDialog
            v-slot="{ section }"
            v-model:tab="fullNarrowTab"
            @reselect="fullReselects += 1"
            :is-open="fullNarrowOpen"
            :overlay-store="appOverlayStore"
            :account="account"
            :sections="sections"
            @close="fullNarrowOpen = false"
          >
            <GallerySettingsFull :key="fullReselects" :tab="section ?? ''" :state="full" />
          </SettingsDialog>
        </GalleryOverlayWell>
      </GallerySection>

      <GallerySection
        title="Skills · Every State"
        note="Every source state and every skill state, pinned. Sources: all on, some on, all off, updating, update available, failed after a good fetch (keeps its skills), first fetch failed (no commit, no skills), skipped files. Skills, in web-kit: on, off, a warning, a taken name, never offered to the agent. Every control acts on the showcase’s own state."
      >
        <GallerySpecimen variant="Wide" wide>
          <div class="w-full rounded-xl border border-line bg-surface-dialog p-6">
            <GallerySkillsShowcase />
          </div>
        </GallerySpecimen>
        <GallerySpecimen variant="Narrow · 360px">
          <div class="w-[360px] max-w-full rounded-xl border border-line bg-surface-dialog p-4">
            <GallerySkillsShowcase />
          </div>
        </GallerySpecimen>
      </GallerySection>

      <GallerySection
        title="Web Search · Every State"
        note="Each state, pinned: nothing connected, so searches go anonymously; signed in with an email; signed in where Exa names none, with a key saved, which searches use; a key saved while Exa’s rate limit kept it from being checked, which the next search decides; a key Exa refused, the typed key kept in the field; and a key typed for Exa to check, whose Save works until Exa answers. Every control acts on its own specimen’s state, and Sign In says what the product would open."
      >
        <GallerySpecimen variant="Not Connected" wide>
          <div class="w-full rounded-xl border border-line bg-surface-dialog p-6">
            <GalleryWebSearchShowcase :state="{ account: null, key: null, inUse: 'anonymous' }" />
          </div>
        </GallerySpecimen>
        <GallerySpecimen variant="Signed In" wide>
          <div class="w-full rounded-xl border border-line bg-surface-dialog p-6">
            <GalleryWebSearchShowcase :state="{ account: { email: 'alex@example.com' }, key: null, inUse: 'account' }" />
          </div>
        </GallerySpecimen>
        <GallerySpecimen variant="Signed In, Key Saved" wide>
          <div class="w-full rounded-xl border border-line bg-surface-dialog p-6">
            <GalleryWebSearchShowcase :state="{ account: { email: null }, key: { lastFour: '7f3a', unchecked: null }, inUse: 'key' }" />
          </div>
        </GallerySpecimen>
        <GallerySpecimen variant="Key Not Checked" wide>
          <div class="w-full rounded-xl border border-line bg-surface-dialog p-6">
            <GalleryWebSearchShowcase
              :state="{ account: null, key: { lastFour: '7f3a', unchecked: 'Exa’s rate limit was reached' }, inUse: 'key' }"
            />
          </div>
        </GallerySpecimen>
        <GallerySpecimen variant="Key Refused" wide>
          <div class="w-full rounded-xl border border-line bg-surface-dialog p-6">
            <GalleryWebSearchShowcase
              :state="{ account: null, key: null, inUse: 'anonymous' }"
              draft="exa_12ab"
              :key-problem="webSearchText('refusals.keyRefused')"
            />
          </div>
        </GallerySpecimen>
        <GallerySpecimen variant="Checking a Key · Press Save" wide>
          <div class="w-full rounded-xl border border-line bg-surface-dialog p-6">
            <GalleryWebSearchShowcase
              :state="{ account: { email: 'alex@example.com' }, key: null, inUse: 'account' }"
              draft="b81c0f2e-55aa-4c3e-9d21-7f3a"
              slow
            />
          </div>
        </GallerySpecimen>
        <GallerySpecimen variant="Narrow · 360px">
          <div class="w-[360px] max-w-full rounded-xl border border-line bg-surface-dialog p-4">
            <GalleryWebSearchShowcase :state="{ account: { email: 'alex@example.com' }, key: { lastFour: '7f3a', unchecked: null }, inUse: 'key' }" />
          </div>
        </GallerySpecimen>
      </GallerySection>

      <GallerySection
        title="Web Search · Calls"
        note="The page over a web plugin of its own. Sign In waits for the plugin, says the product would open Exa’s sign-in page in a new tab, and the account is signed in a moment later, as Exa’s callback would. Save checks the key with one search: one shorter than 16 characters is refused under the field, keeping what was typed; a longer one is saved and shown by its last four characters, and In Use follows. Sign Out and Remove each wait for their call."
      >
        <GallerySpecimen variant="Wide" wide>
          <div class="w-full rounded-xl border border-line bg-surface-dialog p-6">
            <GalleryWebSearchCalls />
          </div>
        </GallerySpecimen>
      </GallerySection>

      <GallerySection
        title="Skills · Calls and Unreadable States"
        note="A control waits for its call’s answer, success or failure, and then shows the plugin’s state again. Turning on web-kit’s review is refused, since review-kit’s review is on: a toast says why and the switch stays off. Backend of an Earlier Build sends the plugin’s state without updateAvailable, as a backend from before that field would: a toast says once, in plain words, that the page cannot read the plugin’s data, the browser console names the missing field, the page keeps the last state it read, and every control still waits only for its own answer."
      >
        <GallerySpecimen variant="Wide" wide>
          <div class="w-full rounded-xl border border-line bg-surface-dialog p-6">
            <GallerySkillsCalls />
          </div>
        </GallerySpecimen>
      </GallerySection>
    </template>

  </div>
</template>
