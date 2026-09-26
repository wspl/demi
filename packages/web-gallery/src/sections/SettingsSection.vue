<script setup lang="ts">
import { ref } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsDialog from '@demicodes/web-ui/settings/SettingsDialog.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import type { SettingsTab } from '@demicodes/web-ui/settings/types'
import GalleryOverlayWell from '../components/GalleryOverlayWell.vue'
import GallerySection from '../components/GallerySection.vue'
import GallerySettingsFull from '../components/GallerySettingsFull.vue'
import { SETTINGS_SECTIONS } from '@demicodes/web-ui/settings/sections'
import { createSettingsState } from '../fixtures/settings'
import { useGalleryView } from '../gallery-views'

const { view } = useGalleryView()

const anatomy: [string, string][] = [
  [
    'Shell',
    'One large dialog. The rail sits on the page surface with the account name on top and a filter under it, the page on the dialog surface, so it reads like the app itself. A whole unused page stays on the rail and is disabled with an In development tooltip: Notifications, MCP servers, Skills, Data & privacy. Below a phone width the rail becomes a row.'
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
    'MCP',
    'The rail entry is disabled with an In development tooltip. The page specimen still shows servers in one list: status is a dot and a word; tools are tags on a third line. Adding opens a dialog.'
  ],
  [
    'Skills',
    'The rail and sidebar entries are disabled with an In development tooltip. The page specimen still shows git sources as packs of SKILL.md files.'
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
const fullTab = ref<SettingsTab>('models')
const fullNarrowTab = ref<SettingsTab>('skills')
// Each pinned dialog closes from its own Close, and Open mounts it again: its
// pages open dialogs of their own into the same well, and patching them back
// into a dialog that closed in place fails in Vue (insertBefore on a node the
// well no longer holds), so a closed dialog is unmounted instead.
const fullOpen = ref(true)
const fullNarrowOpen = ref(true)

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
        title="Full agent settings"
        note="A stress test: expiring auth, an unreachable local model, a crashed MCP server, a quota nearly spent, disabled entries, nested rows, long paths. Visible accounts reuse usage requests for one minute. Automatic refresh keeps existing meters and buttons still and enabled. Only a manual refresh spins its button; it bypasses the TTL or joins an automatic request already running."
      >
        <GalleryOverlayWell size="tall">
          <Button v-if="!fullOpen" size="md" @click="fullOpen = true">Open</Button>
          <SettingsDialog
            v-if="fullOpen"
            v-model:tab="fullTab"
            is-open
            :overlay-store="appOverlayStore"
            :account="account"
            :sections="SETTINGS_SECTIONS"
            @close="fullOpen = false"
          >
            <GallerySettingsFull :tab="fullTab" :state="full" />
          </SettingsDialog>
        </GalleryOverlayWell>
      </GallerySection>

      <GallerySection
        title="Full · narrow"
        note="The long rail becomes a picker; the page still reads at this width."
      >
        <GalleryOverlayWell size="narrow">
          <Button v-if="!fullNarrowOpen" size="md" @click="fullNarrowOpen = true">Open</Button>
          <SettingsDialog
            v-if="fullNarrowOpen"
            v-model:tab="fullNarrowTab"
            is-open
            :overlay-store="appOverlayStore"
            :account="account"
            :sections="SETTINGS_SECTIONS"
            @close="fullNarrowOpen = false"
          >
            <GallerySettingsFull :tab="fullNarrowTab" :state="full" />
          </SettingsDialog>
        </GalleryOverlayWell>
      </GallerySection>
    </template>

  </div>
</template>
