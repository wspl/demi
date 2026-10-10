<script setup lang="ts">
import { demoDeviceInstallation, demoDeviceReport } from '../fixtures/device-installation'
import { DEMO_RUNNER_RELEASE, galleryDevices, useGalleryDevices } from '../fixtures/devices'
import type { CloudState } from '@demicodes/web-ui/cloud/types'
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import type { ThemeChoice } from '@demicodes/web-ui/theme/appTheme'
import {
  applyTranscriptTextSize,
  type ProductAccent,
  type ProductTone,
} from '@demicodes/web-ui/theme/productAppearance'
import { galleryState } from '../gallery-state'
import { productWould } from '../product-would'
import { galleryMessages } from '../fixtures/send-while-running'
import SettingsAccount from '@demicodes/web-ui/settings/SettingsAccount.vue'
import SettingsArchived from '@demicodes/web-ui/settings/SettingsArchived.vue'
import SettingsData from '@demicodes/web-ui/settings/SettingsData.vue'
import SettingsDevices from '@demicodes/web-ui/settings/SettingsDevices.vue'
import SettingsGeneral from '@demicodes/web-ui/settings/SettingsGeneral.vue'
import SettingsKeyboard from '@demicodes/web-ui/settings/SettingsKeyboard.vue'
import { APP_SHORTCUTS } from '@demicodes/web-ui/settings/shortcuts'
import SettingsNotifications from '@demicodes/web-ui/settings/SettingsNotifications.vue'
import SettingsPlugins from '@demicodes/web-ui/settings/SettingsPlugins.vue'
import { PageScope, settingsPage } from '@demicodes/web-ui/plugins/page'
import { PLUGIN_PAGES } from '../generated/pages'
import type { ChangeEmailPhase } from '@demicodes/web-ui/settings/ChangeEmailDialog.vue'
import type { ChangePasswordPhase } from '@demicodes/web-ui/settings/ChangePasswordDialog.vue'
import type { SettingsState } from '../fixtures/settings'
import GallerySettingsProviders from './GallerySettingsProviders.vue'
import GalleryInstructions from './GalleryInstructions.vue'
import GallerySubagents from './GallerySubagents.vue'

/** One page of the full mock, chosen by the dialog's tab. All state lives in the fixture; timers stand in for the server. */
const props = defineProps<{
  tab: string
  state: SettingsState
}>()

const cloud = ref<CloudState>(
  {
    state: 'running',
    operationId: null,
    phase: null,
    error: null,
    volumes: { systemBytes: 6 * 1024 ** 3, homeBytes: 20 * 1024 ** 3 },
    limits: { systemBytes: 16 * 1024 ** 3, homeBytes: 64 * 1024 ** 3 },
    // The server was upgraded since this Cloud's last reset; a reset moves
    // its system to the new image.
    newerImage: true,
    deviceId: 'cloud',
    report: demoDeviceReport('linux', DEMO_RUNNER_RELEASE),
  }
)
// The request is pending until the server accepts it; every second request is refused, so the failed state has a page.
const reset = ref<{ status: 'idle' | 'pending' } | { status: 'failed'; message: string }>({ status: 'idle' })
let resetRequests = 0
async function resetCloud(operationId: string) {
  if (reset.value.status === 'pending')
    return
  reset.value = { status: 'pending' }
  resetRequests += 1
  await new Promise(resolve => setTimeout(resolve, 600))
  if (resetRequests % 2 === 0) {
    reset.value = { status: 'failed', message: 'The Cloud host is not taking resets right now. Try again in a minute.' }
    return
  }
  reset.value = { status: 'idle' }
  // Accepted: the Cloud's status names this reset from now on, as the product's snapshot does.
  cloud.value.operationId = operationId
  cloud.value.state = 'resetting'
  for (const phase of ['stopping', 'saving', 'rebuilding', 'booting', 'ready'] as const) {
    cloud.value.phase = phase
    await new Promise(resolve => setTimeout(resolve, 500))
  }
  cloud.value.state = 'running'
  cloud.value.newerImage = false
}

const s = computed(() => props.state)
/** The page whose section `tab` names, which reaches the section's page host. */
const pluginPage = computed(() => settingsPage(PLUGIN_PAGES, props.tab))

// Text size resizes the transcript here as it does in the product.
watch(() => s.value.general.fontSize, applyTranscriptTextSize, { immediate: true })

/** Light and dark switch the gallery itself, so the preview and the page follow; System keeps the current mode. */
function setThemeChoice(choice: ThemeChoice) {
  s.value.general.theme = choice
  if (choice !== 'system')
    galleryState.mode = choice
}

// Tone and accent change the gallery itself, the way they would change the app. Flat is
// not a product tone yet, so under it the control shows Ink, the product's default.
const tone = computed({
  get: (): ProductTone => (galleryState.paradigm === 'warm' ? 'warm' : 'ink'),
  set: (value: ProductTone) => {
    galleryState.paradigm = value
  },
})
const accent = computed({
  get: () => galleryState.accent as ProductAccent,
  set: (value: ProductAccent) => {
    galleryState.accent = value
  },
})

/**
 * The stand-in browser's permission prompt: it answers what the fixture says
 * after a beat, as a person would; one that blocks the site answers at once,
 * without asking, as a browser does.
 */
async function askBrowser(): Promise<NotificationPermission> {
  const notifications = s.value.notifications
  if (notifications.permission !== 'default') {
    return notifications.permission
  }
  productWould('The Browser Would Ask to Allow Notifications')
  await new Promise((resolve) => setTimeout(resolve, 800))
  notifications.permission = notifications.answer
  return notifications.permission
}

// The field shows the new name at once and waits a beat, as the product's save does.
const nameSaving = ref(false)
let nameTimer = 0
function rename(name: string) {
  s.value.account.name = name
  nameSaving.value = true
  window.clearTimeout(nameTimer)
  nameTimer = window.setTimeout(() => {
    nameSaving.value = false
  }, 600)
}
onBeforeUnmount(() => window.clearTimeout(nameTimer))

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

// A switch shows at once, as the product's does while its write is on its way.
function switchPlugin(id: string, enabled: boolean) {
  const plugin = s.value.plugins.find((item) => item.id === id)
  if (plugin) {
    plugin.enabled = enabled
  }
}

/** A restored or deleted conversation leaves the archived list, as the product's does. */
function restoreArchived(id: string) {
  s.value.archived = s.value.archived.filter((entry) => entry.id !== id)
}

/** The paired devices, whose controls act on them as the product's do. */
const devices = useGalleryDevices(() => galleryDevices())
/** A revoked device leaves the list, and its projects go with it, as the product's do. */
function revokeDevice(id: string) {
  devices.revoke(id)
  s.value.deviceProjects = s.value.deviceProjects.filter((project) => project.deviceId !== id)
}

/** A change the keyboard settings accepted: the row takes the keys, as the product's preference does. */
function rebind(id: string, keys: string) {
  const target = s.value.keys.find((binding) => binding.id === id)
  if (target) {
    target.keys = keys
  }
}

function resetShortcuts() {
  for (const binding of s.value.keys) {
    binding.keys = APP_SHORTCUTS.find((shortcut) => shortcut.id === binding.id)?.keys ?? binding.keys
  }
}
</script>

<template>
  <SettingsGeneral
    v-if="tab === 'general'"
    v-model:language="s.general.language"
    v-model:tone="tone"
    v-model:accent="accent"
    v-model:font-size="s.general.fontSize"
    v-model:send-while-running="galleryMessages.sendWhileRunning"
    :theme="s.general.theme"
    :overlay-store="appOverlayStore"
    :languages="['English', '简体中文', '日本語']"
    @update:theme="setThemeChoice"
  />

  <SettingsAccount
    v-else-if="tab === 'account'"
    :name="s.account.name"
    :name-saving="nameSaving"
    v-model:email-open="emailOpen"
    v-model:password-open="passwordOpen"
    :overlay-store="appOverlayStore"
    :email="s.account.email"
    email-verified
    :password-changed="s.account.passwordChanged"
    :email-phase="emailPhase"
    :password-phase="passwordPhase"
    @update:name="rename"
    @change-email="openChangeEmail"
    @change-password="openChangePassword"
    @submit-email="submitEmail"
    @verify-email="verifyEmail"
    @submit-password="submitPassword"
  />

  <SettingsNotifications
    v-else-if="tab === 'notifications'"
    v-model:enabled="s.notifications.enabled"
    v-model:turn-finishes="s.notifications.turnFinishes"
    v-model:turn-fails="s.notifications.turnFails"
    v-model:needs-permission="s.notifications.needsPermission"
    :permission="s.notifications.permission"
    :request-permission="askBrowser"
  />

  <GallerySettingsProviders v-else-if="tab === 'models'" :state="state" />

  <SettingsPlugins
    v-else-if="tab === 'plugins'"
    :plugins="s.plugins"
    @switch="switchPlugin"
  />

  <GalleryInstructions v-else-if="tab === 'instructions'" v-model="s.instructions" />

  <GallerySubagents v-else-if="tab === 'subagents'" :state="s.subagents" />

  <PageScope
    v-else-if="pluginPage?.settings"
    :page="pluginPage"
    :component="pluginPage.settings.component"
  />

  <SettingsArchived
    v-else-if="tab === 'archived'"
    :conversations="s.archived"
    :overlay-store="appOverlayStore"
    @open="productWould('Open the Conversation Read-Only')"
    @restore="restoreArchived"
    @delete="restoreArchived"
  />

  <SettingsDevices
    v-else-if="tab === 'devices'"
    :cloud="cloud"
    :reset-pending="reset.status === 'pending'"
    :reset-error="reset.status === 'failed' ? reset.message : null"
    @reset-cloud="resetCloud"
    :devices="devices.devices.value"
    :shown="devices.shown.value"
    :runner-release="DEMO_RUNNER_RELEASE"
    :projects="s.deviceProjects"
    :overlay-store="appOverlayStore"
    :installation="demoDeviceInstallation"
    :claim-device="devices.claim"
    :name-max-length="64"
    @show="devices.shown.value = $event"
    @set-route="devices.setRoute"
    @try-now="devices.tryNow"
    @measure="devices.measure"
    @revoke="revokeDevice"
    @rename="devices.rename"
  />

  <SettingsKeyboard
    v-else-if="tab === 'keyboard'"
    :bindings="s.keys"
    @rebind="rebind"
    @reset="resetShortcuts"
  />

  <SettingsData
    v-else-if="tab === 'data'"
    v-model:telemetry="s.data.telemetry"
    @delete-all="productWould('The product would delete all conversations')"
  />

</template>
