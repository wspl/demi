<script setup lang="ts">
import { APP_ICON } from '@demicodes/web-ui/assets/app-icon'
import { t } from './locales/text'
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
import { settingsSections, withUsersSection } from '@demicodes/web-ui/settings/sections'
import { APP_SHORTCUTS, isAppShortcut } from '@demicodes/web-ui/settings/shortcuts'
import type { HeadlineText } from '@demicodes/web-ui/ui/ui-text'
import { PageScope, settingsPage, withPluginSections } from '@demicodes/web-ui/plugins/page'
import { PLUGIN_PAGES } from '../plugins/generated/pages'
import { pluginEnabled } from '../plugins/enabled'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { SerialQueue } from '@demicodes/utils'
import { formError, reportError } from '@demicodes/web-ui/infra/errors'
import { apiRequest, jsonBody, readResponse } from '../api/client'
import {
  emailChangeStartedSchema,
  identitySchema,
  type EmailChangeConfirm,
  type EmailChangeStart,
  type NicknamePatch,
  type PasswordChange,
} from '../api/generated/web-api'
import { useSession } from '../auth/session'
import { usePasskeys } from '../auth/passkeys'
import { useSessions } from '../auth/handoff'
import { useResources } from '../state/resources'
import { useProduct } from '../state/product'
import { usePreferences } from '../state/preferences'
import { useNotifications } from '../state/notifications'
import { useConversations } from '../conversation/store'
import DevicesPanel from './DevicesPanel.vue'
import PortForwardsPanel from './PortForwardsPanel.vue'
import { useSettingsAddress } from './address'
import { CODE_CORRECTIONS, EMAIL_CORRECTIONS, PASSWORD_CORRECTIONS } from './corrections'
import { usePluginSettings } from './plugins'
import AgentPanel from './AgentPanel.vue'
import ProvidersPanel from './ProvidersPanel.vue'
import InstructionsPanel from './InstructionsPanel.vue'
import SubagentsPanel from './SubagentsPanel.vue'
import CredentialsPanel from './CredentialsPanel.vue'
import UsersPanel from './UsersPanel.vue'
import AboutPanel from './AboutPanel.vue'
import type { Action } from '@demicodes/web-ui/ui/operations'

defineProps<{
  /** Signs this browser out; Sign Out works until the page goes to the sign-in page. */
  onSignOut?: Action
}>()
const resources = useResources()
const product = useProduct()
const preferences = usePreferences()
const notifications = useNotifications()
/** Demi's Home Screen icon, which the steps to add Demi to the Home Screen show. */
const homeScreenIcon = APP_ICON
const session = useSession()
const passkeys = usePasskeys()
const sessions = useSessions()
const conversations = useConversations()
const pluginSettings = usePluginSettings()
const router = useRouter()
const lifetime = new AbortController()
/** The rail lists Users for the master and admins only (`users.md` § User system). */
const withUsers = computed(() => withUsersSection(settingsSections(), product.snapshot?.user.role ?? 'user'))
const sections = computed(() =>
  withPluginSections(withUsers.value, PLUGIN_PAGES, (plugin) =>
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

/** The display name's changes on their way, in the order the user made them. */
const nameWrites = new SerialQueue()

/**
 * Changes the display name as the field commits it: the page shows it at
 * once, and a refusal gives the old name back with a toast.
 */
function rename(nickname: string): Promise<void> {
  const next = nickname.trim()
  if (!next) {
    return Promise.resolve()
  }
  const shown = product.change('user', (state) => ({ ...state, user: { ...state.user, nickname: next } }))
  return nameWrites
    .run('nickname', async () => {
      shown.send()
      const response = await apiRequest('/auth/me', {
        method: 'PATCH',
        signal: lifetime.signal,
        ...jsonBody({ nickname: next } satisfies NicknamePatch),
      })
      const { user } = await readResponse(response, identitySchema)
      lifetime.signal.throwIfAborted()
      session.current = {
        status: 'signedIn',
        user,
      }
      shown.land({ type: 'user', user })
    })
    .catch((error) => {
      shown.drop()
      report(t('dialog.nameFailed'), error)
    })
}

/** Why the email cannot be changed on a server without mail (`web-api-settings.md` § Account API). */

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
        report(t('dialog.emailFailed'), error)
      }
      emailPhase.value = {
        ...previous,
        busy: false,
        error: formError(error, EMAIL_CORRECTIONS),
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
        report(t('dialog.emailFailed'), error)
      }
      emailPhase.value = {
        ...previous,
        busy: false,
        error: formError(error, CODE_CORRECTIONS),
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
        report(t('dialog.passwordFailed'), error)
      }
      passwordPhase.value = {
        kind: 'form',
        error: formError(error, PASSWORD_CORRECTIONS),
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

const archived = computed(() =>
  conversations.items
    .filter((conversation) => conversation.archived)
    .map((conversation) => ({
      id: conversation.id,
      title: conversation.title,
    })),
)
/** Restores the conversation, which shows at once, and opens it. */
async function restore(id: string): Promise<void> {
  void conversations.restore([id])
  await openConversation(id)
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
    :account="{ id: resources.userId, name: resources.username, email: resources.email }"
    :sections="sections"
    @close="address.close"
    @reselect="address.show($event)"
  >
    <SettingsGeneral
      v-if="section === 'general'"
      :language="preferences.language"
      :theme="resources.appearance.theme"
      :tone="resources.appearance.tone"
      :accent="resources.appearance.accent"
      :font-size="resources.appearance.fontSize"
      :overlay-store="appOverlayStore"
      @update:language="preferences.update({ language: $event })"
      @update:theme="preferences.update({ appearance: { theme: $event } })"
      @update:tone="preferences.update({ appearance: { tone: $event } })"
      @update:accent="preferences.update({ appearance: { accent: $event } })"
      @update:font-size="
        preferences.update({ appearance: { fontSize: $event } }, { bursty: true })
      "
    />
    <SettingsAccount
      v-else-if="section === 'account'"
      :name="resources.username"
      v-model:email-draft="emailDraft"
      v-model:password-draft="passwordDraft"
      v-model:email-open="emailOpen"
      v-model:password-open="passwordOpen"
      :overlay-store="appOverlayStore"
      :account-id="resources.userId"
      :email="resources.email"
      :email-unavailable="product.snapshot?.mail === false"
      :email-phase="emailPhase"
      :password-phase="passwordPhase"
      :passkeys="passkeys.offered ? passkeys : undefined"
      :sessions="sessions"
      @update:name="rename"
      @change-email="openChangeEmail"
      @change-password="openChangePassword"
      @submit-email="submitEmail"
      @verify-email="verifyEmail"
      @resend-email="resendEmail"
      @submit-password="submitPassword"
      :on-sign-out="onSignOut"
    />
    <SettingsNotifications
      v-else-if="section === 'notifications'"
      v-model:enabled="notifications.settings.enabled"
      v-model:turn-finishes="notifications.settings.turnFinishes"
      v-model:turn-fails="notifications.settings.turnFails"
      v-model:needs-permission="notifications.settings.needsPermission"
      v-model:agent-notifies="notifications.settings.agentNotifies"
      :permission="notifications.permission"
      :request-permission="notifications.requestPermission"
      :home-screen-icon="notifications.homeScreen ? homeScreenIcon : undefined"
      :overlay-store="appOverlayStore"
    />
    <AgentPanel v-else-if="section === 'agent'" />
    <ProvidersPanel v-else-if="section === 'models' && resources.canConfigure" />
    <DevicesPanel v-else-if="section === 'devices'" />
    <PortForwardsPanel v-else-if="section === 'portForwards'" />
    <UsersPanel v-else-if="section === 'users' && product.snapshot?.user.role !== 'user'" />
    <CredentialsPanel v-else-if="section === 'credentials'" />
    <InstructionsPanel v-else-if="section === 'instructions'" />
    <SubagentsPanel v-else-if="section === 'subagents'" />
    <SettingsPlugins
      v-else-if="section === 'plugins'"
      :plugins="product.snapshot?.plugins ?? []"
      @switch="pluginSettings.switchPlugin"
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
      :overlay-store="appOverlayStore"
      @retry="conversations.reloadList"
      @open="openConversation"
      @restore="restore"
      @delete="(id) => void conversations.remove([id])"
    />
    <SettingsKeyboard
      v-else-if="section === 'keyboard'"
      :bindings="resources.keys"
      @rebind="rebind"
      @reset="resetShortcuts"
    />
    <AboutPanel v-else-if="section === 'about'" />
  </SettingsDialog>
</template>
