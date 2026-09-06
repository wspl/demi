<script setup lang="ts">
import { ref } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import ProviderLoginDialog, { type ProviderLoginPhase } from '@demicodes/web-ui/settings/ProviderLoginDialog.vue'
import SettingsDialog from '@demicodes/web-ui/settings/SettingsDialog.vue'
import type { SettingsTab } from '@demicodes/web-ui/settings/types'
import Segmented from '@demicodes/web-ui/ui/Segmented.vue'
import GalleryOverlayWell from '../components/GalleryOverlayWell.vue'
import GallerySection from '../components/GallerySection.vue'
import GallerySettingsFull from '../components/GallerySettingsFull.vue'
import { createSettingsState, fullSettingsNav } from '../fixtures/settings'

const anatomy: [string, string][] = [
  ['Shell', 'One large dialog. The rail sits on the page surface with the account on top and a filter under it, the page on the dialog surface, so it reads like the app itself. Below a phone width the rail becomes a row.'],
  ['Page', 'A title, one line under it, then titled groups. A group is a card of rows: label and explanation left, the control right.'],
  ['Dialogs', 'A dialog that opens from a page keeps its title, search and buttons in place; only the list or form between them scrolls.'],
  ['Readouts', 'A value the control produces (a size, a temperature) reads out beside the control, never in the explanation under the label.'],
  ['Providers', 'A bare rail of providers beside the selected one, with no surface of its own. Every supported subscription is always listed and dotted green once signed in, red when broken; an API key is dotted only while it needs attention. An API-key entry edits its endpoint, key and models on the page; a subscription entry manages accounts. Adding a provider, signing in, and adding or editing a model open dialogs.'],
  ['Sign-in', 'Each subscription signs in the way its vendor does. Claude Code prints a token from its own CLI, so the dialog walks through install, command and paste. Codex confirms a device code in the browser while the dialog waits. Grok Build shows a code the user copies back.'],
]

const account = { name: 'Zan', plan: 'Personal workspace' }
const full = createSettingsState()
const fullTab = ref<SettingsTab>('models')
const fullNarrowTab = ref<SettingsTab>('mcp')

/** Every phase of each vendor's flow, pinned open and switchable. */
const claudePhases: { value: string; label: string; phase: ProviderLoginPhase }[] = [
  { value: 'token', label: 'Token', phase: { kind: 'token', command: 'claude setup-token', install: { label: 'Get Claude Code', url: 'https://docs.anthropic.com/en/docs/claude-code/setup' }, prefix: 'sk-ant-oat01-' } },
  { value: 'done', label: 'Done', phase: { kind: 'done', account: 'zan@example.com · Max 5×' } },
  { value: 'failed', label: 'Failed', phase: { kind: 'failed', message: 'That token was revoked. Run the command again for a fresh one.' } },
]
const codexPhases: { value: string; label: string; phase: ProviderLoginPhase }[] = [
  { value: 'starting', label: 'Starting', phase: { kind: 'starting' } },
  { value: 'device', label: 'Device code', phase: { kind: 'device', url: 'https://auth.openai.com/codex/device', code: 'HXRV-7K2M', expiresIn: '10 min' } },
  { value: 'done', label: 'Done', phase: { kind: 'done', account: 'zan@example.com · Plus' } },
  { value: 'failed', label: 'Failed', phase: { kind: 'failed', message: 'The code expired before it was entered.' } },
]
const grokPhases: { value: string; label: string; phase: ProviderLoginPhase }[] = [
  { value: 'code-input', label: 'Paste code', phase: { kind: 'code-input', url: 'https://accounts.x.ai/device' } },
  { value: 'done', label: 'Done', phase: { kind: 'done', account: 'zan@example.com · SuperGrok' } },
]
const claudePhase = ref('token')
const codexPhase = ref('device')
const grokPhase = ref('code-input')
const phaseOf = (list: { value: string; phase: ProviderLoginPhase }[], value: string) => list.find((p) => p.value === value)!.phase
</script>

<template>
  <div class="flex flex-col gap-10">
    <GallerySection title="Settings" note="The product settings dialog and each of its panels, pinned open with mocked state.">
      <dl class="grid max-w-3xl grid-cols-[7rem_minmax(0,1fr)] gap-x-4 gap-y-2 text-[13px] leading-5">
        <template v-for="[term, detail] in anatomy" :key="term">
          <dt class="select-none text-fg-subtle">{{ term }}</dt>
          <dd class="text-fg-muted">{{ detail }}</dd>
        </template>
      </dl>
    </GallerySection>

    <GallerySection
      title="Full agent settings"
      note="A stress test: everything a coding agent might ask for, in its worst states at once. Expiring auth, an unreachable local model, a crashed MCP server, a quota nearly spent, disabled entries, nested rows, long paths."
    >
      <GalleryOverlayWell size="tall">
        <SettingsDialog v-model:tab="fullTab" :is-open="true" :overlay-store="appOverlayStore" :account="account" :sections="fullSettingsNav">
          <GallerySettingsFull :tab="fullTab" :state="full" />
        </SettingsDialog>
      </GalleryOverlayWell>
    </GallerySection>

    <GallerySection title="Full · narrow" note="The long rail becomes a picker; nested rows and tags survive the width.">
      <GalleryOverlayWell size="narrow">
        <SettingsDialog v-model:tab="fullNarrowTab" :is-open="true" :overlay-store="appOverlayStore" :account="account" :sections="fullSettingsNav">
          <GallerySettingsFull :tab="fullNarrowTab" :state="full" />
        </SettingsDialog>
      </GalleryOverlayWell>
    </GallerySection>

    <GallerySection title="Sign in · Claude Code" note="A token from the vendor's own CLI: install, run, paste.">
      <div class="mb-3"><Segmented v-model="claudePhase" :options="claudePhases" /></div>
      <GalleryOverlayWell size="tall">
        <ProviderLoginDialog :is-open="true" :overlay-store="appOverlayStore" vendor-name="Claude Code" :phase="phaseOf(claudePhases, claudePhase)" />
      </GalleryOverlayWell>
    </GallerySection>

    <GallerySection title="Sign in · Codex" note="A device code confirmed in the browser while the dialog waits.">
      <div class="mb-3"><Segmented v-model="codexPhase" :options="codexPhases" /></div>
      <GalleryOverlayWell size="lg">
        <ProviderLoginDialog :is-open="true" :overlay-store="appOverlayStore" vendor-name="Codex" :phase="phaseOf(codexPhases, codexPhase)" />
      </GalleryOverlayWell>
    </GallerySection>

    <GallerySection title="Sign in · Grok Build" note="A code shown in the browser and pasted back.">
      <div class="mb-3"><Segmented v-model="grokPhase" :options="grokPhases" /></div>
      <GalleryOverlayWell size="lg">
        <ProviderLoginDialog :is-open="true" :overlay-store="appOverlayStore" vendor-name="Grok Build" :phase="phaseOf(grokPhases, grokPhase)" />
      </GalleryOverlayWell>
    </GallerySection>
  </div>
</template>
