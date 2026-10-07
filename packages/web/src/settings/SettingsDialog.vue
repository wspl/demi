<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import SettingsDialog from '@demicodes/web-ui/settings/SettingsDialog.vue'
import SettingsAccount from '@demicodes/web-ui/settings/SettingsAccount.vue'
import SettingsArchived from '@demicodes/web-ui/settings/SettingsArchived.vue'
import SettingsGeneral from '@demicodes/web-ui/settings/SettingsGeneral.vue'
import SettingsKeyboard from '@demicodes/web-ui/settings/SettingsKeyboard.vue'
import SettingsNotifications from '@demicodes/web-ui/settings/SettingsNotifications.vue'
import SettingsPlugins from '@demicodes/web-ui/settings/SettingsPlugins.vue'
import type { ChangeEmailPhase } from '@demicodes/web-ui/settings/ChangeEmailDialog.vue'
import type { ChangePasswordPhase } from '@demicodes/web-ui/settings/ChangePasswordDialog.vue'
import { SETTINGS_SECTIONS } from '@demicodes/web-ui/settings/sections'
import { APP_SHORTCUTS, isAppShortcut } from '@demicodes/web-ui/settings/shortcuts'
import type { HeadlineText } from '@demicodes/web-ui/ui/ui-text'
import { PageScope, settingsPage, withPluginSections } from '@demicodes/web-ui/plugins/page'
import { PLUGIN_PAGES } from '../plugins/generated/pages'
import { pluginEnabled } from '../plugins/enabled'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { reportError } from '@demicodes/web-ui/infra/errors'
import { apiRequest, jsonBody, readResponse } from '../api/client'
import {
  emailChangeStartedSchema,
  identitySchema,
  type EmailChangeConfirm,
  type EmailChangeStart,
  type NicknamePatch,
  type PasswordChange,
  type PluginSwitch,
} from '../api/generated/web-api'
import { useSession } from '../auth/session'
import { useResources } from '../state/resources'
import { useProduct } from '../state/product'
import { usePreferences } from '../state/preferences'
import { useNotifications } from '../state/notifications'
import { useConversations } from '../conversation/store'
import DevicesPanel from './DevicesPanel.vue'
import { useSettingsAddress } from './address'
import ProvidersPanel from './ProvidersPanel.vue'
import SubagentsPanel from './SubagentsPanel.vue'

const emit = defineEmits<{ signOut: [] }>()
const resources = useResources()
const product = useProduct()
const preferences = usePreferences()
const notifications = useNotifications()
const session = useSession()
const conversations = useConversations()
const router = useRouter()
const lifetime = new AbortController()
const sections = computed(() =>
  withPluginSections(SETTINGS_SECTIONS, PLUGIN_PAGES, (plugin) =>
    pluginEnabled(product.snapshot, plugin),
  ).map((group) => ({
    ...group,
    items: group.items.filter(
      (item) => item.id !== 'models' || resources.canConfigure,
    ),
  })),
)
const address = useSettingsAddress()
/** Settings are open while the address is theirs (`address.ts`). */
const open = computed(() => address.section.value !== undefined)
/** The section the address names; null for none chosen. */
const tab = computed({
  get: () => address.section.value ?? null,
  set: (section) => void address.show(section),
})
// An address naming a section the rail does not offer, once the product
// state says which it offers, shows settings with no section chosen.
watch(
  [tab, sections, () => product.snapshot !== null],
  ([section, , known]) => {
    if (section === null || !known) {
      return
    }
    const item = sections.value
      .flatMap((group) => group.items)
      .find((entry) => entry.id === section)
    if (!item || item.disabled) {
      tab.value = null
    }
  },
  { immediate: true },
)

function report(title: HeadlineText, error: unknown): void {
  if (lifetime.signal.aborted) {
    return
  }
  reportError(title, error, { userVisible: true })
}

const nameSaving = ref(false)
const nameDraft = ref<string | null>(null)

async function rename(nickname: string): Promise<void> {
  if (!nickname.trim() || nameSaving.value) {
    return
  }
  nameSaving.value = true
  nameDraft.value = nickname
  try {
    const sentAt = product.sent()
    const response = await apiRequest('/auth/me', {
      method: 'PATCH',
      signal: lifetime.signal,
      ...jsonBody({ nickname: nickname.trim() } satisfies NicknamePatch),
    })
    const { user } = await readResponse(response, identitySchema)
    lifetime.signal.throwIfAborted()
    session.current = {
      status: 'signedIn',
      user,
    }
    product.answered(sentAt, { type: 'user', user })
    nameDraft.value = null
    nameSaving.value = false
  } catch (error) {
    // The field keeps the draft; the toast says why it was not saved.
    nameSaving.value = false
    report('Could Not Change Your Name', error)
  }
}

/** Why the email cannot be changed on a server without mail (`web-api.md` § Account API). */
const EMAIL_UNAVAILABLE = 'This server cannot send email, which a change needs for its code. Ask your administrator.'

const emailDraft = ref({ email: '', password: '', code: '' })
const passwordDraft = ref({ current: '', next: '', confirm: '' })
const emailOpen = ref(false)
const emailPhase = ref<ChangeEmailPhase>({
  kind: 'form',
  currentEmail: '',
})
let emailRequest: AbortController | null = null
let challengeId: string | null = null

function closeEmailRequest(): void {
  emailRequest?.abort()
  emailRequest = null
  challengeId = null
}

function openChangeEmail(): void {
  if (
    emailPhase.value.kind === 'done' ||
    (emailPhase.value.kind === 'form' &&
      !emailPhase.value.busy &&
      !emailPhase.value.error)
  ) {
    emailPhase.value = { kind: 'form', currentEmail: resources.email }
  }
  emailOpen.value = true
}

async function submitEmail(email: string, password: string): Promise<void> {
  if (emailPhase.value.kind === 'done' || emailPhase.value.busy) {
    return
  }
  const previous = emailPhase.value
  const controller = new AbortController()
  emailRequest = controller
  emailPhase.value = {
    ...previous,
    busy: true,
    error: undefined,
  }
  try {
    const response = await apiRequest('/auth/email', {
      method: 'POST',
      ...jsonBody({
        email,
        password,
      } satisfies EmailChangeStart),
      signal: controller.signal,
    })
    const result = await readResponse(response, emailChangeStartedSchema)
    controller.signal.throwIfAborted()
    challengeId = result.challenge.id
    emailPhase.value = {
      kind: 'verify',
      email,
      resent: previous.kind === 'verify',
    }
  } catch (error) {
    if (!controller.signal.aborted) {
      if (!emailOpen.value) {
        report('Could Not Change Email', error)
      }
      emailPhase.value = {
        ...previous,
        busy: false,
        error: error instanceof Error ? error.message : String(error),
      }
    }
  } finally {
    if (emailRequest === controller) {
      emailRequest = null
    }
  }
}

function resendEmail(): void {
  if (emailPhase.value.kind === 'verify') {
    void submitEmail(emailPhase.value.email, emailDraft.value.password)
  }
}

async function verifyEmail(code: string): Promise<void> {
  if (
    !challengeId ||
    emailPhase.value.kind !== 'verify' ||
    emailPhase.value.busy
  ) {
    return
  }
  const previous = emailPhase.value
  const controller = new AbortController()
  emailRequest = controller
  emailPhase.value = {
    ...previous,
    busy: true,
    error: undefined,
  }
  try {
    const sentAt = product.sent()
    const response = await apiRequest('/auth/email/confirm', {
      method: 'POST',
      ...jsonBody({
        id: challengeId,
        code,
      } satisfies EmailChangeConfirm),
      signal: controller.signal,
    })
    const { user } = await readResponse(response, identitySchema)
    controller.signal.throwIfAborted()
    session.current = {
      status: 'signedIn',
      user,
    }
    product.answered(sentAt, { type: 'user', user })
    emailDraft.value = { email: '', password: '', code: '' }
    challengeId = null
    emailPhase.value = {
      kind: 'done',
      email: user.email,
    }
  } catch (error) {
    if (!controller.signal.aborted) {
      if (!emailOpen.value) {
        report('Could Not Change Email', error)
      }
      emailPhase.value = {
        ...previous,
        busy: false,
        error: error instanceof Error ? error.message : String(error),
      }
    }
  } finally {
    if (emailRequest === controller) {
      emailRequest = null
    }
  }
}

const passwordOpen = ref(false)
const passwordPhase = ref<ChangePasswordPhase>({ kind: 'form' })
let passwordRequest: AbortController | null = null

function openChangePassword(): void {
  if (passwordPhase.value.kind === 'done') {
    passwordPhase.value = { kind: 'form' }
  }
  passwordOpen.value = true
}

async function submitPassword(current: string, next: string): Promise<void> {
  if (passwordPhase.value.kind !== 'form' || passwordPhase.value.busy) {
    return
  }
  const controller = new AbortController()
  passwordRequest = controller
  passwordPhase.value = {
    kind: 'form',
    busy: true,
  }
  try {
    await apiRequest('/auth/password', {
      method: 'PUT',
      ...jsonBody({
        current,
        next,
      } satisfies PasswordChange),
      signal: controller.signal,
    })
    controller.signal.throwIfAborted()
    passwordDraft.value = { current: '', next: '', confirm: '' }
    passwordPhase.value = { kind: 'done' }
  } catch (error) {
    if (!controller.signal.aborted) {
      if (!passwordOpen.value) {
        report('Could Not Change Password', error)
      }
      passwordPhase.value = {
        kind: 'form',
        error: error instanceof Error ? error.message : String(error),
      }
    }
  } finally {
    if (passwordRequest === controller) {
      passwordRequest = null
    }
  }
}

watch(
  open,
  (isOpen) => {
    if (!isOpen) {
      emailOpen.value = false
      passwordOpen.value = false
    }
  },
)
onUnmounted(() => {
  lifetime.abort()
  closeEmailRequest()
  passwordRequest?.abort()
})

/** The switches asked for and not yet in the product state, by plugin. */
const wantedPlugins = ref(new Map<string, boolean>())
const plugins = computed(() =>
  (product.snapshot?.plugins ?? []).map((plugin) => ({
    ...plugin,
    enabled: wantedPlugins.value.get(plugin.id) ?? plugin.enabled,
  })),
)

/**
 * Turns a plugin on or off; the switch shows the choice until the channel
 * brings the plugin list that holds it, or falls back with a toast.
 */
async function switchPlugin(id: string, enabled: boolean): Promise<void> {
  if (wantedPlugins.value.has(id)) {
    return
  }
  wantedPlugins.value.set(id, enabled)
  try {
    await apiRequest(`/plugins/${encodeURIComponent(id)}`, {
      method: 'PUT',
      signal: lifetime.signal,
      ...jsonBody({ enabled } satisfies PluginSwitch),
    })
    await product.until(
      (state) => state.plugins.some((plugin) => plugin.id === id && plugin.enabled === enabled),
      lifetime.signal,
    )
  } catch (error) {
    report(enabled ? 'Could Not Turn the Plugin On' : 'Could Not Turn the Plugin Off', error)
  } finally {
    wantedPlugins.value.delete(id)
  }
}

const archived = computed(() =>
  conversations.items
    .filter((conversation) => conversation.archived)
    .map((conversation) => ({
      id: conversation.id,
      title: conversation.title,
    })),
)
async function restore(id: string): Promise<void> {
  if (conversations.pendingChanges.includes(id)) {
    return
  }
  if (await conversations.restore([id])) {
    await openConversation(id)
  }
}
/** An archived conversation opens read-only, with the bar that offers Restore; leaving settings' address closes them. */
async function openConversation(id: string): Promise<void> {
  await router.push(`/chat/${id}`)
}

function rebind(id: string, keys: string): void {
  if (isAppShortcut(id)) {
    preferences.update({ shortcuts: { [id]: keys } })
  }
}
function resetShortcuts(): void {
  preferences.update({
    shortcuts: Object.fromEntries(APP_SHORTCUTS.map((shortcut) => [shortcut.id, null])),
  })
}
</script>

<template>
  <SettingsDialog
    v-slot="{ section }"
    v-model:tab="tab"
    :is-open="open"
    :overlay-store="appOverlayStore"
    :account="{ name: resources.username, email: resources.email }"
    :sections="sections"
    @close="address.close"
  >
    <SettingsGeneral
      v-if="section === 'general'"
      language="English"
      :theme="resources.appearance.theme"
      :tone="resources.appearance.tone"
      :accent="resources.appearance.accent"
      :font-size="resources.appearance.fontSize"
      :overlay-store="appOverlayStore"
      :languages="['English']"
      @update:theme="preferences.update({ appearance: { theme: $event } })"
      @update:tone="preferences.update({ appearance: { tone: $event } })"
      @update:accent="preferences.update({ appearance: { accent: $event } })"
      @update:font-size="
        preferences.update({ appearance: { fontSize: $event } })
      "
    />
    <SettingsAccount
      v-else-if="section === 'account'"
      :name="nameDraft ?? resources.username"
      :name-saving="nameSaving"
      v-model:email-draft="emailDraft"
      v-model:password-draft="passwordDraft"
      v-model:email-open="emailOpen"
      v-model:password-open="passwordOpen"
      :overlay-store="appOverlayStore"
      :email="resources.email"
      :email-unavailable="product.snapshot?.mail === false ? EMAIL_UNAVAILABLE : undefined"
      :email-phase="emailPhase"
      :password-phase="passwordPhase"
      @update:name="rename"
      @change-email="openChangeEmail"
      @change-password="openChangePassword"
      @submit-email="submitEmail"
      @verify-email="verifyEmail"
      @resend-email="resendEmail"
      @submit-password="submitPassword"
      @sign-out="emit('signOut')"
    />
    <SettingsNotifications
      v-else-if="section === 'notifications'"
      v-model:enabled="notifications.settings.enabled"
      v-model:turn-finishes="notifications.settings.turnFinishes"
      v-model:turn-fails="notifications.settings.turnFails"
      v-model:needs-permission="notifications.settings.needsPermission"
      :permission="notifications.permission"
      :request-permission="notifications.requestPermission"
    />
    <ProvidersPanel v-else-if="section === 'models' && resources.canConfigure" />
    <DevicesPanel v-else-if="section === 'devices'" />
    <SubagentsPanel v-else-if="section === 'subagents'" />
    <SettingsPlugins
      v-else-if="section === 'plugins'"
      :plugins="plugins"
      :pending="[...wantedPlugins.keys()]"
      @switch="switchPlugin"
    />
    <PageScope
      v-else-if="section !== null && settingsPage(PLUGIN_PAGES, section)?.settings"
      :page="settingsPage(PLUGIN_PAGES, section)!"
      :component="settingsPage(PLUGIN_PAGES, section)!.settings!.component"
    />
    <SettingsArchived
      v-else-if="section === 'archived'"
      :conversations="archived"
      :load="conversations.listStatus"
      :pending-ids="conversations.pendingChanges"
      @retry="conversations.reloadList"
      @open="openConversation"
      @restore="restore"
    />
    <SettingsKeyboard
      v-else-if="section === 'keyboard'"
      :bindings="resources.keys"
      @rebind="rebind"
      @reset="resetShortcuts"
    />
  </SettingsDialog>
</template>
