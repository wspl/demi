<script setup lang="ts">
import { ref } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import ChangeEmailDialog, { type ChangeEmailPhase } from '@demicodes/web-ui/settings/ChangeEmailDialog.vue'
import ChangePasswordDialog, { type ChangePasswordPhase } from '@demicodes/web-ui/settings/ChangePasswordDialog.vue'
import ProviderLoginDialog, { type ProviderLoginPhase } from '@demicodes/web-ui/settings/ProviderLoginDialog.vue'
import SettingsDialog from '@demicodes/web-ui/settings/SettingsDialog.vue'
import type { SettingsTab } from '@demicodes/web-ui/settings/types'
import Segmented from '@demicodes/web-ui/ui/Segmented.vue'
import GalleryOverlayWell from '../components/GalleryOverlayWell.vue'
import GallerySection from '../components/GallerySection.vue'
import GallerySettingsFull from '../components/GallerySettingsFull.vue'
import GalleryDevicePairing from '../components/GalleryDevicePairing.vue'
import { SETTINGS_SECTIONS } from '@demicodes/web-ui/settings/sections'
import { createSettingsState } from '../fixtures/settings'

const anatomy: [string, string][] = [
  ['Shell', 'One large dialog. The rail sits on the page surface with the account on top and a filter under it, the page on the dialog surface, so it reads like the app itself. Below a phone width the rail becomes a row.'],
  ['Page', 'A title, one line under it, then titled groups. A group is a card of rows: label and explanation left, the control right.'],
  ['Dialogs', 'A dialog that opens from a page keeps its title, search and buttons in place; only the list or form between them scrolls.'],
  ['Readouts', 'A value the control produces (a size, a temperature) reads out beside the control, never in the explanation under the label.'],
  ['Providers', 'A bare rail of providers beside the selected one, with no surface of its own. Every supported subscription is always listed and dotted green once signed in, red when broken; an API key is dotted only while it needs attention. An API-key entry edits its endpoint, key and models on the page; a subscription entry manages accounts. Adding a provider, signing in, and adding or editing a model open dialogs.'],
  ['Credentials', 'Changing the email asks for the new address and the current password, then a code sent to the new address. Changing the password asks for the current one and the new one twice; length and the match are checked in the dialog, the current password by the server.'],
  ['MCP', 'Servers in one list. Status is a dot and a word; an error sits on the status as a tooltip. Tools are tags on a third line, one row, with the rest as +N. Adding opens a dialog.'],
  ['Skills', 'Git repositories, not a marketplace. One source is a pack of SKILL.md files. Packs start folded with a Fold animation; more than six skills scroll inside the pack. The source row has no hover wash. Each skill has a switch. The pack’s switch is on when any skill is on and sets every skill. Beside it: N/N only when mixed. Adding is a git URL. skills.sh is a link, not a catalog.'],
  ['Sign-in', 'Each subscription signs in the way its vendor does. Claude Code prints a token from its own CLI, so the dialog walks through install, command and paste. Codex confirms a device code in the browser while the dialog waits. Grok Build shows a code the user copies back.'],
]

const account = { name: 'Zan', plan: 'Personal workspace' }
const full = createSettingsState()
const fullTab = ref<SettingsTab>('models')
const fullNarrowTab = ref<SettingsTab>('skills')

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

const emailPhases: { value: string; label: string; phase: ChangeEmailPhase }[] = [
  { value: 'form', label: 'Form', phase: { kind: 'form', currentEmail: 'zan@example.com' } },
  { value: 'form-error', label: 'Wrong password', phase: { kind: 'form', currentEmail: 'zan@example.com', error: 'That is not your current password.' } },
  { value: 'verify', label: 'Verify', phase: { kind: 'verify', email: 'zan@demi.codes' } },
  { value: 'verify-error', label: 'Wrong code', phase: { kind: 'verify', email: 'zan@demi.codes', error: 'That code is not right. Check the newest message.' } },
  { value: 'done', label: 'Done', phase: { kind: 'done', email: 'zan@demi.codes' } },
]
const emailPhase = ref('form')
const passwordPhases: { value: string; label: string; phase: ChangePasswordPhase }[] = [
  { value: 'form', label: 'Form', phase: { kind: 'form' } },
  { value: 'error', label: 'Wrong password', phase: { kind: 'form', error: 'That is not your current password.' } },
  { value: 'done', label: 'Done', phase: { kind: 'done' } },
]
const passwordPhase = ref('form')
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
        <SettingsDialog v-model:tab="fullTab" :is-open="true" :overlay-store="appOverlayStore" :account="account" :sections="SETTINGS_SECTIONS">
          <GallerySettingsFull :tab="fullTab" :state="full" />
        </SettingsDialog>
      </GalleryOverlayWell>
    </GallerySection>

    <GallerySection title="Full · narrow" note="The long rail becomes a picker; the page still reads at this width.">
      <GalleryOverlayWell size="narrow">
        <SettingsDialog v-model:tab="fullNarrowTab" :is-open="true" :overlay-store="appOverlayStore" :account="account" :sections="SETTINGS_SECTIONS">
          <GallerySettingsFull :tab="fullNarrowTab" :state="full" />
        </SettingsDialog>
      </GalleryOverlayWell>
    </GallerySection>

    <GallerySection title="Add device · pairing flow" note="One shared flow for a local computer, a remote SSH server, or a headless machine. Start the runner, paste its pairing code, then use the connected device.">
      <GalleryDevicePairing />
    </GallerySection>

    <GallerySection title="Change email" note="The new address and the current password, then the code that proves the address is reachable.">
      <div class="mb-3"><Segmented v-model="emailPhase" :options="emailPhases" /></div>
      <GalleryOverlayWell size="tall">
        <ChangeEmailDialog :is-open="true" :overlay-store="appOverlayStore" :phase="emailPhases.find((p) => p.value === emailPhase)!.phase" />
      </GalleryOverlayWell>
    </GallerySection>

    <GallerySection title="Change password" note="The current password and the new one twice.">
      <div class="mb-3"><Segmented v-model="passwordPhase" :options="passwordPhases" /></div>
      <GalleryOverlayWell size="tall">
        <ChangePasswordDialog :is-open="true" :overlay-store="appOverlayStore" :phase="passwordPhases.find((p) => p.value === passwordPhase)!.phase" />
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
