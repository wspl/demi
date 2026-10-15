<script setup lang="ts">
import { t as uiText } from '../locales/text'
import { t } from './locales/text'
import { formError, type ShownError } from '../infra/errors'
import { computed, ref } from 'vue'
import { Eye, EyeOff } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import ConfirmDialog from '../ui/ConfirmDialog.vue'
import IconButton from '../ui/IconButton.vue'
import RelativeTime from '../ui/RelativeTime.vue'
import Tag from '../ui/Tag.vue'
import { formatMoment } from '../ui/dates'
import CredentialDialog from './CredentialDialog.vue'
import { answerInDialog } from './dialog-work'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import type { SettingsCredential, SettingsCredentialDraft, SettingsCredentialField } from './types'

/**
 * The Credentials section (`credentials.md` § A credential): the user's
 * credentials by name, each with its fields, a tag of each field's kind,
 * *Kept until* and *Last used*, and an editor. A visible field shows its value
 * as text, as the list carries it. A secret field shows dots and an eye that
 * reads its value from the host and shows it to the user; that value goes
 * again as the pointer or the focus leaves the row, and is not kept anywhere
 * else. The host saves and deletes; the next read of an agent uses a change.
 */
const props = defineProps<{
  credentials: SettingsCredential[]
  overlayStore: OverlayStore
  /** Creates a credential when `id` is null, else changes that one; a refusal is thrown for the editor to show. */
  saveCredential: (id: string | null, draft: SettingsCredentialDraft) => Promise<void>
  deleteCredential: (id: string) => Promise<void>
  /**
   * The values of a credential's fields by name, for the eye of a secret
   * field; null when they could not be read, which the host has told the user.
   */
  revealCredential: (id: string) => Promise<Record<string, string> | null>
}>()

interface Editor {
  /** The credential edited; null for a new one. */
  credential: SettingsCredential | null
  open: boolean
  /** Why the last save or delete was refused, shown in the dialog. */
  error: ShownError | null
}

const editor = ref<Editor | null>(null)

function open(credential: SettingsCredential | null): void {
  editor.value = { credential, open: true, error: null }
}

function failed(error: unknown): ShownError {
  // A refused name or field is the user's to correct in the dialog.
  return formError(error, { invalid_body: null, credential_name_taken: null })
}

/** The credential as the target of its Save and Delete, one group of operations (`operations.ts`). */
function credentialTarget(id: string | null): string {
  return `credential:${id ?? 'new'}`
}

/** The dialog of credential `id`, if it is the one open: the answer to its work goes there and nowhere else. */
function shownEditor(id: string | null): Editor | null {
  const current = editor.value
  return current?.open && (current.credential?.id ?? null) === id ? current : null
}

function save(draft: SettingsCredentialDraft): Promise<void> {
  const id = editor.value?.credential?.id ?? null
  return answerInDialog(() => shownEditor(id), t('credentials.saveFailed'), failed, () => props.saveCredential(id, draft))
}

/** Deleting asks first, over the credential's dialog. */
const deleteOpen = ref(false)

async function remove(): Promise<void> {
  const id = editor.value?.credential?.id
  if (!id) {
    return
  }
  await answerInDialog(() => shownEditor(id), t('credentials.deleteFailed'), failed, () => props.deleteCredential(id))
  // The question closes with its answer, if it still asks about this credential.
  if (editor.value?.credential?.id === id) {
    deleteOpen.value = false
  }
}

/** Save and Delete exclude each other on one credential: while one works, the other cannot be pressed. */
function editing(action: 'save' | 'delete', id: string | null) {
  return { action, target: credentialTarget(id), group: 'edit' }
}

/** What the user has revealed, by credential: its values and the secret fields shown. */
const revealed = ref(new Map<string, { values: Record<string, string>; shown: Set<string> }>())
async function toggle(credential: SettingsCredential, field: string): Promise<void> {
  const held = revealed.value.get(credential.id)
  if (held?.shown.has(field)) {
    held.shown.delete(field)
    return
  }
  if (held) {
    held.shown.add(field)
    return
  }
  const values = await props.revealCredential(credential.id)
  if (values) {
    revealed.value = new Map(revealed.value).set(credential.id, { values, shown: new Set([field]) })
  }
}

/** Hides what the row showed: the values are dropped, not kept out of sight. */
function hide(id: string): void {
  if (revealed.value.has(id)) {
    const next = new Map(revealed.value)
    next.delete(id)
    revealed.value = next
  }
}

function leaveRow(event: FocusEvent, id: string): void {
  if (!(event.currentTarget as HTMLElement).contains(event.relatedTarget as Node | null)) {
    hide(id)
  }
}

/** What the row shows of a field: a visible field's value as listed, a secret's only after its own eye; null while it is hidden. */
function fieldValue(credential: SettingsCredential, field: SettingsCredentialField): string | null {
  if (field.kind === 'visible') {
    return field.value
  }
  const held = revealed.value.get(credential.id)
  if (!held || !held.shown.has(field.name)) {
    return null
  }
  return held.values[field.name] ?? ''
}

const empty = computed(() => props.credentials.length === 0)
</script>

<template>
  <SettingsPage :title="t('credentials.title')">
    <SettingsGroup :title="t('credentials.yours')">
      <template #actions>
        <Button size="sm" @click="open(null)">{{ t('credentials.new') }}</Button>
      </template>
      <div
        v-for="credential in credentials"
        :key="credential.id"
        @pointerleave="hide(credential.id)"
        @focusout="leaveRow($event, credential.id)"
      >
        <SettingsRow
          :label="credential.name"
          :description="credential.note || undefined"
          :target="credentialTarget(credential.id)"
          interactive
          isolate-controls
          @click="open(credential)"
        >
          <template #detail>
            <div class="flex flex-col gap-1.5">
              <!-- One grid for the fields, so the labels, the tags and the values line up row to row. -->
              <div class="grid grid-cols-[max-content_max-content_minmax(0,1fr)] items-center gap-x-2 gap-y-1 text-[12px] leading-4">
                <template v-for="field in credential.fields" :key="field.name">
                  <span class="min-h-5 content-center text-fg-body">{{ field.label }}</span>
                  <Tag>{{ field.kind === 'secret' ? t('credentials.secret') : t('credentials.visible') }}</Tag>
                  <span class="flex min-h-5 min-w-0 items-center gap-1.5">
                    <template v-if="field.kind === 'secret'">
                      <span
                        class="min-w-0 truncate font-mono text-fg-muted"
                        :class="fieldValue(credential, field) === null ? 'select-none' : ''"
                      >{{ fieldValue(credential, field) ?? '••••••••' }}</span>
                      <IconButton
                        :icon="fieldValue(credential, field) === null ? Eye : EyeOff"
                        size="xs"
                        variant="ghost"
                        :aria-label="t(fieldValue(credential, field) === null ? 'credentials.show' : 'credentials.hide', { label: field.label })"
                        @click.stop="toggle(credential, field.name)"
                      />
                    </template>
                    <span
                      v-else-if="fieldValue(credential, field) !== null"
                      class="min-w-0 truncate font-mono text-fg-muted"
                    >{{ fieldValue(credential, field) }}</span>
                  </span>
                </template>
              </div>
              <div class="flex flex-wrap gap-x-4 gap-y-0.5 text-[12px] leading-4 text-fg-subtle">
                <span>
                  {{ t('credentials.keptUntil') }}
                  <span class="text-fg-muted">{{ credential.keepUntil ? formatMoment(credential.keepUntil) : t('credentials.always') }}</span>
                </span>
                <span>
                  {{ t('credentials.lastUsed') }}
                  <span class="text-fg-muted">
                    <RelativeTime v-if="credential.lastUsedAt" :timestamp="credential.lastUsedAt" />
                    <template v-else>{{ t('credentials.never') }}</template>
                  </span>
                </span>
              </div>
            </div>
          </template>
          <Button size="sm" @click="open(credential)">{{ t('credentials.edit') }}</Button>
        </SettingsRow>
      </div>
      <div
        v-if="empty"
        class="select-none px-4 py-6 text-center text-[13px] text-fg-subtle"
      >
        {{ t('credentials.none') }}
      </div>
    </SettingsGroup>

    <CredentialDialog
      v-if="editor"
      :is-open="editor.open"
      :overlay-store="overlayStore"
      :credential="editor.credential"
      :error="editor.error"
      :save-operation="editing('save', editor.credential?.id ?? null)"
      :delete-operation="editing('delete', editor.credential?.id ?? null)"
      @close="editor.open = false"
      @save="save"
      @delete="deleteOpen = true"
    />
    <ConfirmDialog
      :is-open="deleteOpen"
      :overlay-store="overlayStore"
      :title="t('credentials.deleteTitle', { name: editor?.credential?.name ?? '' })"
      :action="uiText('common.delete')"
      :operation="editing('delete', editor?.credential?.id ?? null)"
      @close="deleteOpen = false"
      @confirm="remove"
    >
      <p>{{ t('credentials.deleteBody') }}</p>
    </ConfirmDialog>
  </SettingsPage>
</template>
