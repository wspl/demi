<script setup lang="ts">
import { t as uiText } from '../locales/text'
import { t } from './locales/text'
import { formError, reportError, type ShownError } from '../infra/errors'
import { computed, ref } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import { Trash2 } from '@lucide/vue'
import Button from '../ui/Button.vue'
import ConfirmDialog from '../ui/ConfirmDialog.vue'
import IconButton from '../ui/IconButton.vue'
import Tooltip from '../ui/Tooltip.vue'
import RelativeTime from '../ui/RelativeTime.vue'
import TruncatedText from '../ui/TruncatedText.vue'
import { formatMoment } from '../ui/dates'
import CredentialDialog from './CredentialDialog.vue'
import { answerInDialog, type WorkDialog } from './dialog-work'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import type { SettingsCredential, SettingsCredentialDraft } from './types'

/**
 * The Credentials section: the user's credentials by name, each a row of three
 * lines and a Delete… icon, as a list row's actions are icons:
 *
 * ```
 * Linear sign-in                                             [trash icon]
 * To file issues in the team's tracker from a conversa…
 * User name  alex@example.com    Kept until Always · Last used 2 days ago
 * ```
 *
 * - The note, the text the credential was asked for with, is one line, cut
 *   with an ellipsis.
 * - A visible field shows its label and value, as the list carries them. A
 *   secret field shows nothing: its value never reaches the page.
 * - A credential is not edited. Delete… asks first; New Credential… creates one.
 * - The host saves and deletes; the next read of an agent uses a change.
 */
const props = defineProps<{
  credentials: SettingsCredential[]
  overlayStore: OverlayStore
  /** Creates a credential; a refusal is thrown for New Credential to show. */
  createCredential: (draft: SettingsCredentialDraft) => Promise<void>
  /** Deletes a credential; a failure is thrown for the page to report. */
  deleteCredential: (id: string) => Promise<void>
}>()

/** New Credential, while it is shown; null before it is first opened. */
const creating = ref<WorkDialog | null>(null)

function openNew(): void {
  creating.value = { open: true, error: null }
}

function failed(error: unknown): ShownError {
  // A refused name or field is the user's to correct in the dialog.
  return formError(error, { invalid_body: null, credential_name_taken: null })
}

/** Creating, as an operation: New Credential's Create Credential shows it. */
const creation = { action: 'create', target: 'credential:new' }

function create(draft: SettingsCredentialDraft): Promise<void> {
  return answerInDialog(
    () => (creating.value?.open ? creating.value : null),
    t('credentials.saveFailed'),
    failed,
    () => props.createCredential(draft),
  )
}

/** The credential Delete… asks about, kept while the question closes. */
const deleting = ref<SettingsCredential | null>(null)
const deleteOpen = ref(false)

function askDelete(credential: SettingsCredential): void {
  deleting.value = credential
  deleteOpen.value = true
}

/** Deleting a credential, as an operation on it: its Delete… shows it while it runs. */
function deletion(credential: SettingsCredential | null) {
  return { action: 'delete', target: `credential:${credential?.id ?? ''}` }
}

/** Delete confirmed: once it is done, the question closes, if it still asks about this credential. */
async function remove(): Promise<void> {
  const credential = deleting.value
  if (!credential) {
    return
  }
  try {
    await props.deleteCredential(credential.id)
  } catch (error) {
    reportError(t('credentials.deleteFailed'), error, { userVisible: true })
  } finally {
    if (deleting.value?.id === credential.id) {
      deleteOpen.value = false
    }
  }
}

/** The fields a row shows: a secret one shows nothing at all. */
function visibleFields(credential: SettingsCredential) {
  return credential.fields.filter((field) => field.kind === 'visible')
}

const empty = computed(() => props.credentials.length === 0)
</script>

<template>
  <SettingsPage :title="t('credentials.title')">
    <SettingsGroup :title="t('credentials.yours')">
      <template #actions>
        <Button size="sm" @click="openNew">{{ t('credentials.new') }}</Button>
      </template>
      <SettingsRow
        v-for="credential in credentials"
        :key="credential.id"
        :label="credential.name"
      >
        <template v-if="credential.note" #description>
          <TruncatedText :text="credential.note" />
        </template>
        <template #detail>
          <div class="flex flex-wrap justify-between gap-x-4 gap-y-0.5 text-setting-small leading-4 text-fg-subtle phone:leading-5">
            <!-- The fields keep together at the line's start, the times go to its end. -->
            <span v-if="visibleFields(credential).length" class="flex min-w-0 max-w-full flex-wrap gap-x-4 gap-y-0.5">
              <span
                v-for="field in visibleFields(credential)"
                :key="field.name"
                class="flex min-w-0 max-w-full gap-2"
              >
                <span class="shrink-0">{{ field.label }}</span>
                <TruncatedText class="text-fg-muted" :text="field.value ?? ''" />
              </span>
            </span>
            <span>
              {{ t('credentials.keptUntil') }}
              <span class="text-fg-muted">{{ credential.keepUntil ? formatMoment(credential.keepUntil) : t('credentials.always') }}</span>
              ·
              {{ t('credentials.lastUsed') }}
              <span class="text-fg-muted">
                <RelativeTime v-if="credential.lastUsedAt" :timestamp="credential.lastUsedAt" />
                <template v-else>{{ t('credentials.never') }}</template>
              </span>
            </span>
          </div>
        </template>
        <Tooltip :content="t('credentials.delete')"
          ><IconButton
            size="sm"
            :icon="Trash2"
            variant="danger"
            :aria-label="t('credentials.deleteLabel', { name: credential.name })"
            :operation="deletion(credential)"
            @click="askDelete(credential)"
        /></Tooltip>
      </SettingsRow>
      <div
        v-if="empty"
        class="select-none px-4 py-6 text-center text-[13px] text-fg-subtle"
      >
        {{ t('credentials.none') }}
      </div>
    </SettingsGroup>

    <CredentialDialog
      v-if="creating"
      :is-open="creating.open"
      :overlay-store="overlayStore"
      :error="creating.error"
      :save-operation="creation"
      @close="creating.open = false"
      @save="create"
    />
    <ConfirmDialog
      :is-open="deleteOpen"
      :overlay-store="overlayStore"
      :title="t('credentials.deleteTitle', { name: deleting?.name ?? '' })"
      :action="uiText('common.delete')"
      :operation="deletion(deleting)"
      @close="deleteOpen = false"
      @confirm="remove"
    >
      <p>{{ t('credentials.deleteBody') }}</p>
    </ConfirmDialog>
  </SettingsPage>
</template>
