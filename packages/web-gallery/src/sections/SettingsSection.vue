<script setup lang="ts">
import { computed, ref } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsDialog from '@demicodes/web-ui/settings/SettingsDialog.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import type { SettingsTab } from '@demicodes/web-ui/settings/types'
import GalleryOverlayWell from '../components/GalleryOverlayWell.vue'
import GallerySection from '../components/GallerySection.vue'
import GallerySettingsFull from '../components/GallerySettingsFull.vue'
import GallerySkillsShowcase from '../components/GallerySkillsShowcase.vue'
import GallerySkillsCalls from '../components/GallerySkillsCalls.vue'
import GallerySpecimen from '../components/GallerySpecimen.vue'
import { SETTINGS_SECTIONS } from '@demicodes/web-ui/settings/sections'
import { createSettingsState } from '../fixtures/settings'
import { galleryPageHost, skillsPlugin } from '../fixtures/plugins'
import { PLUGIN_PAGES } from '../generated/pages'
import { providePageHost, withPluginSections } from '@demicodes/web-ui/plugins/page'
import { useGalleryView } from '../gallery-views'
import GallerySubagents from '../components/GallerySubagents.vue'
import SubagentProfileDialog from '@demicodes/web-ui/settings/SubagentProfileDialog.vue'
import type { SettingsSubagentDraft } from '@demicodes/web-ui/settings/types'
import {
  createSubagentState,
  demoSubagentProfiles,
  subagentModels,
  subagentProviders,
} from '../fixtures/subagent-profiles'
import { productWould } from '../product-would'

const { view } = useGalleryView()

const anatomy: [string, string][] = [
  [
    'Shell',
    'One large dialog. The rail sits on the page surface with the account name on top and a filter under it, the page on the dialog surface, so it reads like the app itself. A whole unused page stays on the rail and is disabled with an In development tooltip: Notifications, Data & privacy. A plugin\'s section is on the rail while its plugin is on. On a narrow screen, where the app\'s side panes become overlays, the dialog fills the window: square corners, no scrim around it. Below a phone width the rail becomes a row.'
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
    'Cloud shows its shared-environment description and its storage: each filesystem\'s current size and the most it may grow to, or only the most before its first start. There is no connection or lifecycle status. Reset progress and errors belong to the reset dialog.'
  ],
  [
    'Providers',
    'A bare rail of providers beside the selected one, with no surface of its own. Every supported subscription is always listed. A dot means attention, for every kind of provider alike: none while it works, yellow while it is not set up, red when it failed or cannot be reached. An API-key entry edits its endpoint, key and models on the page; a subscription entry manages accounts. Adding a provider, signing in, and adding or editing a model open dialogs.'
  ],
  [
    'Credentials',
    'Changing the email asks for the new address and the current password, then a code sent to the new address. Changing the password asks for the current one and the new one twice; length and the match are checked in the dialog, the current password by the server.'
  ],
  [
    'Subagent',
    'A switch that turns subagents on or off for the user, then the user\'s profiles, each with its own switch. A row names the profile\'s model, its prompt and whether its children spawn; a profile whose provider entry, model, effort or tier is gone carries Unavailable, with the missing part as its tooltip. The editor chooses the parent\'s model or any provider\'s with its effort and tier, and the parent\'s prompt or text that replaces it.'
  ],
  [
    'Skills',
    'A plugin\'s section: the rail lists it under Agent, and the sidebar has an entry that opens it, both only while the Skills plugin is on. Git sources are packs of SKILL.md files; a source\'s state is one status label after its name, and a skill\'s labels say why it is off or never offered. The showcase below pins every state at once.'
  ],
  [
    'Data',
    'The rail entry is disabled with an In development tooltip because every control on the page is deferred. The page specimen still shows retention, share links, export, diagnostics and delete-all.'
  ],
  [
    'Sign-in',
    'Each subscription signs in the way its vendor does. Claude Code prints a token from its own CLI, so the dialog walks through install, command and paste. Codex and Grok Build confirm a device code in the browser while the dialog waits.'
  ],
]

const account = { name: 'Zan' }
const full = createSettingsState()
// The plugins' sections reach the fixture's skills plugin, and show while the
// fixture's Plugins page has their plugin on, as the product's do.
providePageHost(galleryPageHost({ skills: skillsPlugin(full.skills) }))
const sections = computed(() =>
  withPluginSections(SETTINGS_SECTIONS, PLUGIN_PAGES, (plugin) =>
    full.plugins.some((entry) => entry.id === plugin && entry.enabled),
  ),
)
const fullTab = ref<SettingsTab>('models')
const fullNarrowTab = ref<SettingsTab>('skills')
// Each pinned dialog closes from its own Close and opens again in place from Open,
// as the product's settings dialog does.
const fullOpen = ref(true)
const fullNarrowOpen = ref(true)

// The Subagent section with subagents turned off: the profiles stay, muted.
const subagentsOff = createSubagentState(false)

/** A profile pinned in its editor, which closes from its buttons and opens again from Open. */
function pinnedEditor(id: string | null) {
  const profile = demoSubagentProfiles().find((candidate) => candidate.id === id)
  const draft: SettingsSubagentDraft = profile
    ? {
        name: profile.name,
        description: profile.description,
        model: profile.model,
        instructions: profile.instructions,
        canSpawn: profile.canSpawn,
      }
    : { name: '', description: '', model: null, instructions: null, canSpawn: true }
  return { open: ref(true), draft }
}
const inheritEditor = pinnedEditor(null)
const ownEditor = pinnedEditor('explore')

function saved(editor: ReturnType<typeof pinnedEditor>, draft: SettingsSubagentDraft) {
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
        title="Full Agent Settings"
        note="A stress test: expiring auth, an unreachable local model, a quota nearly spent, disabled entries, nested rows, long paths. Visible accounts reuse usage requests for one minute. Automatic refresh keeps existing meters and buttons still and enabled. Only a manual refresh spins its button; it bypasses the TTL or joins an automatic request already running."
      >
        <GalleryOverlayWell size="tall">
          <Button v-if="!fullOpen" size="md" @click="fullOpen = true">Open</Button>
          <SettingsDialog
            v-model:tab="fullTab"
            :is-open="fullOpen"
            :overlay-store="appOverlayStore"
            :account="account"
            :sections="sections"
            @close="fullOpen = false"
          >
            <GallerySettingsFull :tab="fullTab" :state="full" />
          </SettingsDialog>
        </GalleryOverlayWell>
      </GallerySection>

      <GallerySection
        title="Subagent · Off"
        note="Subagents turned off: no agent spawns, the profiles stay and can still be edited. The disabled archivist's model is gone too, and the scout's effort is no longer offered."
      >
        <div class="max-w-3xl rounded-xl border border-line bg-surface p-6">
          <GallerySubagents :state="subagentsOff" />
        </div>
      </GallerySection>

      <GallerySection
        title="Profile Editor · Inherits"
        note="A new profile: the parent's model and prompt, its children may spawn. Create Profile stays off until the name and the description are filled."
      >
        <GalleryOverlayWell size="tall">
          <Button v-if="!inheritEditor.open.value" size="md" @click="inheritEditor.open.value = true">Open</Button>
          <SubagentProfileDialog
            :is-open="inheritEditor.open.value"
            :overlay-store="appOverlayStore"
            mode="create"
            :profile="inheritEditor.draft"
            :providers="subagentProviders"
            :models="subagentModels"
            @close="inheritEditor.open.value = false"
            @save="saved(inheritEditor, $event)"
          />
        </GalleryOverlayWell>
      </GallerySection>

      <GallerySection
        title="Profile Editor · Own Prompt, Another Provider’s Model"
        note="The explore profile: GPT-5 of OpenAI with Fast, while the conversations run on Anthropic, a prompt that replaces the parent's, and no spawning."
      >
        <GalleryOverlayWell size="tall">
          <Button v-if="!ownEditor.open.value" size="md" @click="ownEditor.open.value = true">Open</Button>
          <SubagentProfileDialog
            :is-open="ownEditor.open.value"
            :overlay-store="appOverlayStore"
            mode="edit"
            :profile="ownEditor.draft"
            :providers="subagentProviders"
            :models="subagentModels"
            @close="ownEditor.open.value = false"
            @save="saved(ownEditor, $event)"
            @delete="deleted(ownEditor)"
          />
        </GalleryOverlayWell>
      </GallerySection>

      <GallerySection
        title="Full · Narrow"
        note="A phone-width window: the dialog fills it edge to edge with square corners and no scrim around it, the close button stays in its corner, and the long rail becomes a picker."
      >
        <GalleryOverlayWell size="narrow">
          <Button v-if="!fullNarrowOpen" size="md" @click="fullNarrowOpen = true">Open</Button>
          <SettingsDialog
            v-model:tab="fullNarrowTab"
            :is-open="fullNarrowOpen"
            :overlay-store="appOverlayStore"
            :account="account"
            :sections="sections"
            @close="fullNarrowOpen = false"
          >
            <GallerySettingsFull :tab="fullNarrowTab" :state="full" />
          </SettingsDialog>
        </GalleryOverlayWell>
      </GallerySection>

      <GallerySection
        title="Skills · Every State"
        note="Every source state and every skill state, pinned. Sources: all on, some on, all off, updating, update available, failed after a good fetch (keeps its skills), first fetch failed (no commit, no skills), skipped files. Skills, in web-kit: on, off, a warning, a taken name, never offered to the agent. Every control acts on the showcase's own state."
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
        title="Skills · Calls and Unreadable States"
        note="A control waits for its call's answer, success or failure, and then shows the plugin's state again. Turning on web-kit's review is refused, since review-kit's review is on: a toast says why and the switch stays off. Backend of an Earlier Build sends the plugin's state without updateAvailable, as a backend from before that field would: a toast says once, in plain words, that the page cannot read the plugin's data, the browser console names the missing field, the page keeps the last state it read, and every control still waits only for its own answer."
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
