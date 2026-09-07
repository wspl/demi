<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import SettingsDialog from '@demicodes/web-ui/settings/SettingsDialog.vue'
import SettingsAccount from '@demicodes/web-ui/settings/SettingsAccount.vue'
import SettingsArchived from '@demicodes/web-ui/settings/SettingsArchived.vue'
import SettingsData from '@demicodes/web-ui/settings/SettingsData.vue'
import SettingsGeneral from '@demicodes/web-ui/settings/SettingsGeneral.vue'
import SettingsKeyboard from '@demicodes/web-ui/settings/SettingsKeyboard.vue'
import SettingsMcp from '@demicodes/web-ui/settings/SettingsMcp.vue'
import SettingsNotifications from '@demicodes/web-ui/settings/SettingsNotifications.vue'
import SettingsSkills from '@demicodes/web-ui/settings/SettingsSkills.vue'
import type { ChangeEmailPhase } from '@demicodes/web-ui/settings/ChangeEmailDialog.vue'
import type { ChangePasswordPhase } from '@demicodes/web-ui/settings/ChangePasswordDialog.vue'
import type { SettingsMcpDraft, SettingsMcpServer, SettingsSkillDraft, SettingsSkillSource, SettingsTab } from '@demicodes/web-ui/settings/types'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { showToast } from '@demicodes/web-ui/infra/toast'
import { setThemeChoice } from '@demicodes/web-ui/theme/appTheme'
import { applyProductAppearance, applyTranscriptTextSize } from '@demicodes/web-ui/theme/productAppearance'
import { useResources } from '../prototype/resources'
import { useConversations } from '../conversation/store'
import { DEFAULT_KEYS, LANGUAGES, RETENTIONS } from '../prototype/settings'
import DevicesPanel from './DevicesPanel.vue'
import ProvidersPanel from './ProvidersPanel.vue'

/**
 * The product's settings over the prototype's state. Pages are shared; what a row
 * does to the app (the theme, the shortcuts, the conversations) is decided here.
 */
const emit = defineEmits<{ signOut: [] }>()
const resources = useResources()
const conversations = useConversations()
const router = useRouter()
const s = resources.settings

const tab = computed({
  get: () => resources.settingsTab as SettingsTab,
  set: (value: SettingsTab) => {
    resources.settingsTab = value
  },
})

// General: every choice lands on the document the moment it changes.
watch(() => s.general.theme, setThemeChoice)
watch(() => [s.general.tone, s.general.accent] as const, ([tone, accent]) => applyProductAppearance({ tone, accent }))
watch(() => s.general.fontSize, applyTranscriptTextSize)

// Account: the server would check the password and send the code; here a beat stands in.
const emailOpen = ref(false)
const emailPhase = ref<ChangeEmailPhase>({ kind: 'form', currentEmail: '' })
let emailTimer = 0
function openChangeEmail() {
  window.clearTimeout(emailTimer)
  emailPhase.value = { kind: 'form', currentEmail: s.account.email }
  emailOpen.value = true
}
function submitEmail(email: string, password: string) {
  if (emailPhase.value.kind !== 'form') return
  if (password === 'wrong') {
    emailPhase.value = { ...emailPhase.value, error: 'That is not your current password.' }
    return
  }
  emailPhase.value = { ...emailPhase.value, busy: true, error: undefined }
  emailTimer = window.setTimeout(() => {
    emailPhase.value = { kind: 'verify', email }
  }, 700)
}
function verifyEmail(code: string) {
  if (emailPhase.value.kind !== 'verify') return
  const { email } = emailPhase.value
  emailPhase.value = { kind: 'verify', email, busy: true }
  emailTimer = window.setTimeout(() => {
    if (code === '000000') {
      emailPhase.value = { kind: 'verify', email, error: 'That code is not right. Check the newest message.' }
      return
    }
    s.account.email = email
    emailPhase.value = { kind: 'done', email }
  }, 600)
}

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
      passwordPhase.value = { kind: 'form', error: 'That is not your current password.' }
      return
    }
    s.account.passwordChanged = 'just now'
    passwordPhase.value = { kind: 'done' }
  }, 600)
}

/** The prototype keeps no account server-side: deleting is signing out with nothing to come back to. */
function deleteAccount() {
  conversations.items = []
  resources.projects = []
  resources.devices = []
  emit('signOut')
}

// Archived: restoring brings the conversation back and opens it in place of the settings.
const archived = computed(() => conversations.items.filter((c) => c.archived).map((c) => ({ id: c.id, title: c.title })))
function restore(id: string) {
  conversations.archive([id], false)
  resources.settingsOpen = false
  void router.push(`/chat/${id}`)
}

// MCP: adding connects at once; a restart or sign-in clears the fault.
function addServer(draft: SettingsMcpDraft) {
  s.servers.push({ id: `server-${Date.now()}`, ...draft, state: 'connected', enabled: true, tools: [] })
}
function reconnectServer(server: SettingsMcpServer) {
  server.state = 'connected'
  server.detail = undefined
}

// Skills: a source is a git repository; the prototype discovers two example skills in any of them.
function addSkillSource(draft: SettingsSkillDraft) {
  const id = `src-${Date.now()}`
  s.skillSources.push({
    id,
    name: draft.origin.replace(/^https?:\/\/github\.com\//, '').replace(/\.git$/, ''),
    origin: draft.origin,
    state: 'ready',
    skills: [
      { id: `${id}-one`, name: 'example-one', description: 'A skill discovered in this repository.', enabled: true },
      { id: `${id}-two`, name: 'example-two', description: 'Another skill from the same pack.', enabled: true },
    ],
  })
}
function removeSkillSource(source: SettingsSkillSource) {
  s.skillSources = s.skillSources.filter((entry) => entry.id !== source.id)
}
function updateSkillSource(source: SettingsSkillSource) {
  source.state = 'updating'
  window.setTimeout(() => {
    source.state = 'ready'
  }, 600)
}

// Keyboard: a binding another action holds is refused; the row keeps its old keys.
const keyMessage = ref('')
function rebind(id: string, keys: string) {
  const target = s.keys.find((binding) => binding.id === id)
  if (!target) return
  const taken = s.keys.find((binding) => binding.id !== id && binding.keys === keys)
  if (taken) {
    keyMessage.value = `${keys} is already bound to “${taken.action}”.`
    return
  }
  keyMessage.value = ''
  target.keys = keys
}
function resetShortcuts() {
  for (const binding of s.keys) binding.keys = DEFAULT_KEYS.find((entry) => entry.id === binding.id)?.keys ?? binding.keys
  keyMessage.value = ''
}

// Data: the export has no surface of its own until it is ready; deleting empties the list in place.
function requestExport() {
  showToast({ title: 'Export requested', message: 'A link arrives by email when the archive is ready.' })
}
function deleteAllConversations() {
  conversations.items = []
  resources.settingsOpen = false
  void router.push('/chat')
}

</script>

<template>
  <SettingsDialog
    v-model:tab="tab"
    :is-open="resources.settingsOpen"
    :overlay-store="appOverlayStore"
    :account="{ name: resources.username || 'Zan', plan: `${s.account.plan} plan` }"
    @close="resources.settingsOpen = false"
  >
    <SettingsGeneral
      v-if="tab === 'general'"
      v-model:language="s.general.language"
      v-model:theme="s.general.theme"
      v-model:tone="s.general.tone"
      v-model:accent="s.general.accent"
      v-model:font-size="s.general.fontSize"
      :overlay-store="appOverlayStore"
      :languages="LANGUAGES"
    />
    <SettingsAccount
      v-else-if="tab === 'account'"
      v-model:name="resources.username"
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
      @sign-out="emit('signOut')"
      @delete-account="deleteAccount"
    />
    <SettingsNotifications
      v-else-if="tab === 'notifications'"
      v-model:browser="s.notifications.browser"
      v-model:sound="s.notifications.sound"
      v-model:on-finish="s.notifications.onFinish"
      v-model:on-error="s.notifications.onError"
    />
    <ProvidersPanel v-else-if="tab === 'models'" />
    <SettingsMcp v-else-if="tab === 'mcp'" :servers="s.servers" :overlay-store="appOverlayStore" @add="addServer" @sign-in="reconnectServer" @restart="reconnectServer" />
    <SettingsSkills v-else-if="tab === 'skills'" :sources="s.skillSources" :overlay-store="appOverlayStore" @add="addSkillSource" @remove="removeSkillSource" @update="updateSkillSource" />
    <DevicesPanel v-else-if="tab === 'devices'" />
    <SettingsArchived v-else-if="tab === 'archived'" :conversations="archived" @restore="restore" />
    <SettingsKeyboard v-else-if="tab === 'keyboard'" :bindings="s.keys" :message="keyMessage" @rebind="rebind" @reset="resetShortcuts" />
    <SettingsData
      v-else-if="tab === 'data'"
      v-model:retention="s.data.retention"
      v-model:share-links="s.data.shareLinks"
      v-model:telemetry="s.data.telemetry"
      :overlay-store="appOverlayStore"
      :retentions="RETENTIONS"
      @export="requestExport"
      @delete-all="deleteAllConversations"
    />
  </SettingsDialog>
</template>
