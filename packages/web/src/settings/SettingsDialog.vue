<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import { z } from 'zod'
import SettingsDialog from '@demicodes/web-ui/settings/SettingsDialog.vue'
import SettingsAccount from '@demicodes/web-ui/settings/SettingsAccount.vue'
import SettingsArchived from '@demicodes/web-ui/settings/SettingsArchived.vue'
import SettingsGeneral from '@demicodes/web-ui/settings/SettingsGeneral.vue'
import SettingsKeyboard from '@demicodes/web-ui/settings/SettingsKeyboard.vue'
import type { ChangeEmailPhase } from '@demicodes/web-ui/settings/ChangeEmailDialog.vue'
import type { ChangePasswordPhase } from '@demicodes/web-ui/settings/ChangePasswordDialog.vue'
import { SETTINGS_SECTIONS } from '@demicodes/web-ui/settings/sections'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { reportError } from '@demicodes/web-ui/infra/errors'
import { apiRequest, jsonBody, readResponse } from '../api/client'
import { identitySchema } from '../api/contracts'
import { useSession } from '../auth/session'
import { useResources } from '../state/resources'
import { useProduct } from '../state/product'
import { usePreferences } from '../state/preferences'
import { useConversations } from '../conversation/store'
import DevicesPanel from './DevicesPanel.vue'
import ProvidersPanel from './ProvidersPanel.vue'

const emit = defineEmits<{ signOut: [] }>()
const resources = useResources()
const product = useProduct()
const preferences = usePreferences()
const session = useSession()
const conversations = useConversations()
const router = useRouter()
const lifetime = new AbortController()
const sections = computed(() =>
  SETTINGS_SECTIONS.map((group) => ({
    ...group,
    items: group.items.filter(
      (item) => item.id !== 'models' || resources.canConfigure,
    ),
  })),
)
const tab = computed({
  get: () => resources.settingsTab,
  set: (value) => {
    resources.settingsTab = value
  },
})
watch(
  [tab, sections],
  () => {
    const item = sections.value
      .flatMap((group) => group.items)
      .find((item) => item.id === tab.value)
    if (!item || item.disabled) {
      tab.value = 'general'
    }
  },
  { immediate: true },
)

function report(title: string, error: unknown): void {
  if (lifetime.signal.aborted) {
    return
  }
  reportError(title, error, { userVisible: true })
}

const nameSave = ref<'idle' | 'saving' | 'saved'>('idle')
const nameDraft = ref<string | null>(null)

async function rename(nickname: string): Promise<void> {
  if (!nickname.trim() || nameSave.value === 'saving') {
    return
  }
  nameSave.value = 'saving'
  nameDraft.value = nickname
  try {
    const response = await apiRequest('/auth/me', {
      method: 'PATCH',
      signal: lifetime.signal,
      ...jsonBody({ nickname: nickname.trim() }),
    })
    const { user } = await readResponse(response, identitySchema)
    lifetime.signal.throwIfAborted()
    session.current = {
      status: 'signedIn',
      user,
    }
    if (product.snapshot) {
      product.snapshot.user = user
    }
    nameDraft.value = null
    nameSave.value = 'saved'
    await product.revalidate()
  } catch (error) {
    // The field keeps the draft; the toast says why it was not saved.
    nameSave.value = 'idle'
    report('Could not change your name', error)
  }
}

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
      }),
      signal: controller.signal,
    })
    const result = await readResponse(
      response,
      z.object({
        challenge: z.object({ id: z.string().min(1) }),
      }),
    )
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
        report('Could not change email', error)
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
    const response = await apiRequest('/auth/email/confirm', {
      method: 'POST',
      ...jsonBody({
        id: challengeId,
        code,
      }),
      signal: controller.signal,
    })
    const { user } = await readResponse(response, identitySchema)
    controller.signal.throwIfAborted()
    session.current = {
      status: 'signedIn',
      user,
    }
    if (product.snapshot) {
      product.snapshot.user = user
    }
    emailDraft.value = { email: '', password: '', code: '' }
    challengeId = null
    emailPhase.value = {
      kind: 'done',
      email: user.email,
    }
    await product.revalidate()
  } catch (error) {
    if (!controller.signal.aborted) {
      if (!emailOpen.value) {
        report('Could not change email', error)
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
      }),
      signal: controller.signal,
    })
    controller.signal.throwIfAborted()
    passwordDraft.value = { current: '', next: '', confirm: '' }
    passwordPhase.value = { kind: 'done' }
  } catch (error) {
    if (!controller.signal.aborted) {
      if (!passwordOpen.value) {
        report('Could not change password', error)
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
  () => resources.settingsOpen,
  (open) => {
    if (!open) {
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
async function restore(id: string): Promise<void> {
  if (conversations.pendingChanges.includes(id)) {
    return
  }
  if (await conversations.archive([id], false)) {
    resources.settingsOpen = false
    await router.push(`/chat/${id}`)
  }
}

const keyMessage = ref('')
function rebind(id: string, keys: string): void {
  if (id !== 'new' && id !== 'sidebar' && id !== 'settings') {
    return
  }
  const taken = resources.keys.find(
    (binding) => binding.id !== id && keys !== '' && binding.keys === keys,
  )
  if (taken) {
    keyMessage.value = `${keys} is already bound to “${taken.action}”.`
    return
  }
  keyMessage.value = ''
  preferences.update({ shortcuts: { [id]: keys } })
}
function resetShortcuts(): void {
  keyMessage.value = ''
  preferences.update({
    shortcuts: {
      new: null,
      sidebar: null,
      settings: null,
    },
  })
}
</script>

<template>
  <SettingsDialog
    v-model:tab="tab"
    :is-open="resources.settingsOpen"
    :overlay-store="appOverlayStore"
    :account="{ name: resources.username, email: resources.email }"
    :sections="sections"
    @close="resources.settingsOpen = false"
  >
    <SettingsGeneral
      v-if="tab === 'general'"
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
      v-else-if="tab === 'account'"
      :name="nameDraft ?? resources.username"
      :name-save="nameSave"
      v-model:email-draft="emailDraft"
      v-model:password-draft="passwordDraft"
      v-model:email-open="emailOpen"
      v-model:password-open="passwordOpen"
      :overlay-store="appOverlayStore"
      :email="resources.email"
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
    <ProvidersPanel v-else-if="tab === 'models' && resources.canConfigure" />
    <DevicesPanel v-else-if="tab === 'devices'" />
    <SettingsArchived
      v-else-if="tab === 'archived'"
      :conversations="archived"
      :load="conversations.listStatus"
      :pending-ids="conversations.pendingChanges"
      @retry="conversations.reloadList"
      @restore="restore"
    />
    <SettingsKeyboard
      v-else-if="tab === 'keyboard'"
      :bindings="resources.keys"
      :message="keyMessage"
      @rebind="rebind"
      @reset="resetShortcuts"
    />
  </SettingsDialog>
</template>
