<script setup lang="ts">
import { demoDeviceInstallation } from '../fixtures/device-installation'
import type { CloudState } from '@demicodes/web-ui/cloud/types'
import { computed, ref, watch } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import type { ThemeChoice } from '@demicodes/web-ui/theme/appTheme'
import {
  applyTranscriptTextSize,
  type ProductAccent,
  type ProductTone,
} from '@demicodes/web-ui/theme/productAppearance'
import { galleryState } from '../gallery-state'
import SettingsAccount from '@demicodes/web-ui/settings/SettingsAccount.vue'
import SettingsArchived from '@demicodes/web-ui/settings/SettingsArchived.vue'
import SettingsData from '@demicodes/web-ui/settings/SettingsData.vue'
import SettingsDevices from '@demicodes/web-ui/settings/SettingsDevices.vue'
import SettingsGeneral from '@demicodes/web-ui/settings/SettingsGeneral.vue'
import SettingsKeyboard from '@demicodes/web-ui/settings/SettingsKeyboard.vue'
import SettingsMcp from '@demicodes/web-ui/settings/SettingsMcp.vue'
import SettingsNotifications from '@demicodes/web-ui/settings/SettingsNotifications.vue'
import SettingsSkills from '@demicodes/web-ui/settings/SettingsSkills.vue'
import type { SettingsMcpDraft, SettingsSkillDraft, SettingsSkillSource } from '@demicodes/web-ui/settings/types'
import type { ChangeEmailPhase } from '@demicodes/web-ui/settings/ChangeEmailDialog.vue'
import type { ChangePasswordPhase } from '@demicodes/web-ui/settings/ChangePasswordDialog.vue'
import type { SettingsState } from '../fixtures/settings'
import GallerySettingsProviders from './GallerySettingsProviders.vue'

/** One page of the full mock, chosen by the dialog's tab. All state lives in the fixture; timers stand in for the server. */
const props = defineProps<{
  tab: string
  state: SettingsState
}>()

const cloud = ref<CloudState>(
  {
    state: 'running',
    phase: null,
    error: null,
    systemBytes: 16 * 1024 ** 3,
    homeBytes: 32 * 1024 ** 3
  }
)
// The request is pending until the server accepts it; every second request is refused, so the failed state has a page.
const reset = ref<{ status: 'idle' | 'pending' } | { status: 'failed'; message: string }>({ status: 'idle' })
let resetRequests = 0
async function resetCloud() {
  if (reset.value.status === 'pending')
    return
  reset.value = { status: 'pending' }
  resetRequests += 1
  await new Promise(resolve => setTimeout(resolve, 600))
  if (resetRequests % 2 === 0) {
    reset.value = { status: 'failed', message: 'HTTP 503: the cloud host is not accepting operations right now.' }
    return
  }
  reset.value = { status: 'idle' }
  cloud.value.state = 'resetting'
  for (const phase of ['stopping', 'saving', 'rebuilding', 'booting', 'ready'] as const) {
    cloud.value.phase = phase
    await new Promise(resolve => setTimeout(resolve, 500))
  }
  cloud.value.state = 'running'
}

const s = computed(() => props.state)

// Text size resizes the transcript here as it does in the product.
watch(() => s.value.general.fontSize, applyTranscriptTextSize, { immediate: true })

/** Light and dark switch the gallery itself, so the preview and the page follow; System keeps the current mode. */
function setThemeChoice(choice: ThemeChoice) {
  s.value.general.theme = choice
  if (choice !== 'system')
    galleryState.mode = choice
}

// Tone and accent change the gallery itself, the way they would change the app.
const tone = computed({
  get: () => galleryState.tone as ProductTone,
  set: (value: ProductTone) => {
    galleryState.tone = value
  },
})
const accent = computed({
  get: () => galleryState.accent as ProductAccent,
  set: (value: ProductAccent) => {
    galleryState.accent = value
  },
})

// Email: the code goes out after a beat; 000000 is the one code that is wrong.
const emailOpen = ref(false)
const emailPhase = ref<ChangeEmailPhase>({ kind: 'form', currentEmail: '' })
let emailTimer = 0
function openChangeEmail() {
  window.clearTimeout(emailTimer)
  emailPhase.value = { kind: 'form', currentEmail: s.value.account.email }
  emailOpen.value = true
}
function submitEmail(email: string, password: string) {
  if (emailPhase.value.kind !== 'form')
    return
  if (password === 'wrong') {
    emailPhase.value = {
      ...emailPhase.value,
      error: 'That is not your current password.'
    }
    return
  }
  emailPhase.value = { ...emailPhase.value, busy: true, error: undefined }
  emailTimer = window.setTimeout(() => {
    emailPhase.value = { kind: 'verify', email }
  }, 700)
}
function verifyEmail(code: string) {
  if (emailPhase.value.kind !== 'verify')
    return
  const { email } = emailPhase.value
  emailPhase.value = { kind: 'verify', email, busy: true }
  emailTimer = window.setTimeout(() => {
    if (code === '000000') {
      emailPhase.value = {
        kind: 'verify',
        email,
        error: 'That code is not right. Check the newest message.'
      }
      return
    }
    s.value.account.email = email
    emailPhase.value = { kind: 'done', email }
  }, 600)
}

// Password: "wrong" is the one current password that is not accepted.
const passwordOpen = ref(false)
const passwordPhase = ref<ChangePasswordPhase>({ kind: 'form' })
let passwordTimer = 0
function openChangePassword() {
  window.clearTimeout(passwordTimer)
  passwordPhase.value = { kind: 'form' }
  passwordOpen.value = true
}
function submitPassword(current: string) {
  passwordPhase.value = { kind: 'form', busy: true }
  passwordTimer = window.setTimeout(() => {
    if (current === 'wrong') {
      passwordPhase.value = {
        kind: 'form',
        error: 'That is not your current password.'
      }
      return
    }
    s.value.account.passwordChanged = 'just now'
    passwordPhase.value = { kind: 'done' }
  }, 600)
}

function addServer(draft: SettingsMcpDraft) {
  s.value.servers.push(
    {
      id: `server-${Date.now()}`,
      ...draft,
      state: 'connected',
      enabled: true,
      tools: []
    }
  )
}

function restartServer(server: SettingsState['servers'][number]) {
  server.state = 'connected'
  server.detail = undefined
}

function signInServer(server: SettingsState['servers'][number]) {
  server.state = 'connected'
  server.detail = undefined
}

function sourceName(origin: string) {
  return origin.replace(/^https?:\/\/github\.com\//, '').replace(/\.git$/, '')
}

function addSkillSource(draft: SettingsSkillDraft) {
  const id = `src-${Date.now()}`
  const name = sourceName(draft.origin)
  s.value.skillSources.push({
    id,
    name,
    origin: draft.origin,
    state: 'ready',
    skills: [
      {
        id: `${id}-one`,
        name: 'example-one',
        description: 'A skill discovered in this repository.',
        enabled: true
      },
      {
        id: `${id}-two`,
        name: 'example-two',
        description: 'Another skill from the same pack.',
        enabled: true
      },
    ],
  })
}

function removeSkillSource(source: SettingsSkillSource) {
  s.value.skillSources = s.value.skillSources.filter((entry) => entry.id !== source.id)
}

function updateSkillSource(source: SettingsSkillSource) {
  source.state = 'updating'
  window.setTimeout(() => {
    source.state = 'ready'
  }, 600)
}

function restoreArchived(id: string) {
  s.value.archived = s.value.archived.filter((entry) => entry.id !== id)
}

function revokeDevice(id: string) {
  s.value.devices = s.value.devices.filter((d) => d.id !== id)
}

async function claimDevice(_code: string) {
  await new Promise((resolve) => window.setTimeout(resolve, 900))
  const n = s.value.devices.length + 1
  const device = {
    id: `device-${Date.now()}`,
    name: `host-${n}`,
    online: true,
    seen: 'Now'
  }
  s.value.devices.push(device)
  return { ok: true as const, device }
}

const keyMessage = ref('')

/** A binding another action already holds is refused; the row keeps its old keys. */
function rebind(id: string, keys: string) {
  const list = s.value.keys
  const target = list.find((b) => b.id === id)
  if (!target)
    return
  const taken = list.find((b) => b.id !== id && b.keys === keys)
  if (taken) {
    keyMessage.value = `${keys} is already bound to “${taken.action}”.`
    return
  }
  keyMessage.value = ''
  target.keys = keys
}

const DEFAULT_KEYS: Record<string, string> = {
  new: '⌘⇧O',
  send: '⏎',
  stop: '⎋',
  sidebar: '⌘B',
  search: '⌘K',
  focus: '⌘J',
  settings: '⌘,'
}
function resetShortcuts() {
  for (const binding of s.value.keys) binding.keys = DEFAULT_KEYS[binding.id] ?? binding.keys
  keyMessage.value = ''
}

</script>

<template>
  <SettingsGeneral
    v-if="tab === 'general'"
    v-model:language="s.general.language"
    v-model:tone="tone"
    v-model:accent="accent"
    v-model:font-size="s.general.fontSize"
    :theme="s.general.theme"
    :overlay-store="appOverlayStore"
    :languages="['English', '简体中文', '日本語']"
    @update:theme="setThemeChoice"
  />

  <SettingsAccount
    v-else-if="tab === 'account'"
    v-model:name="s.account.name"
    v-model:email-open="emailOpen"
    v-model:password-open="passwordOpen"
    :overlay-store="appOverlayStore"
    :email="s.account.email"
    email-verified
    :password-changed="s.account.passwordChanged"
    :email-phase="emailPhase"
    :password-phase="passwordPhase"
    @change-email="openChangeEmail"
    @change-password="openChangePassword"
    @submit-email="submitEmail"
    @verify-email="verifyEmail"
    @submit-password="submitPassword"
  />

  <SettingsNotifications
    v-else-if="tab === 'notifications'"
    v-model:browser="s.notifications.browser"
    v-model:sound="s.notifications.sound"
    v-model:on-finish="s.notifications.onFinish"
    v-model:on-error="s.notifications.onError"
  />

  <GallerySettingsProviders v-else-if="tab === 'models'" :state="state" />

  <SettingsMcp
    v-else-if="tab === 'mcp'"
    :servers="s.servers"
    :overlay-store="appOverlayStore"
    @add="addServer"
    @sign-in="signInServer"
    @restart="restartServer"
  />

  <SettingsSkills
    v-else-if="tab === 'skills'"
    :sources="s.skillSources"
    :overlay-store="appOverlayStore"
    @add="addSkillSource"
    @remove="removeSkillSource"
    @update="updateSkillSource"
  />

  <SettingsArchived
    v-else-if="tab === 'archived'"
    :conversations="s.archived"
    @restore="restoreArchived"
  />

  <SettingsDevices
    v-else-if="tab === 'devices'"
    :cloud="cloud"
    :reset-pending="reset.status === 'pending'"
    :reset-error="reset.status === 'failed' ? reset.message : null"
    @reset-cloud="resetCloud"
    :devices="s.devices"
    :overlay-store="appOverlayStore"
    :installation="demoDeviceInstallation"
    :claim-device="claimDevice"
    @revoke="revokeDevice"
  />

  <SettingsKeyboard
    v-else-if="tab === 'keyboard'"
    :bindings="s.keys"
    :message="keyMessage"
    @rebind="rebind"
    @reset="resetShortcuts"
  />

  <SettingsData
    v-else-if="tab === 'data'"
    v-model:retention="s.data.retention"
    v-model:share-links="s.data.shareLinks"
    v-model:telemetry="s.data.telemetry"
    :overlay-store="appOverlayStore"
    :retentions="['Forever', '90 days', '30 days', '7 days']"
  />

</template>
