<script setup lang="ts">
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
  status: { kind: 'idle' | 'saving' } | ({ kind: 'failed' } & ShownError)
}

const editor = ref<Editor | null>(null)

function open(credential: SettingsCredential | null): void {
  editor.value = { credential, open: true, status: { kind: 'idle' } }
}

function failed(error: unknown): Editor['status'] {
  // A refused name or field is the user's to correct in the dialog.
  return { kind: 'failed', ...formError(error, { invalid_body: null, credential_name_taken: null }) }
}

async function save(draft: SettingsCredentialDraft): Promise<void> {
  const current = editor.value
  if (!current || current.status.kind === 'saving') {
    return
  }
  current.status = { kind: 'saving' }
  try {
    await props.saveCredential(current.credential?.id ?? null, draft)
    current.status = { kind: 'idle' }
    current.open = false
  } catch (error) {
    current.status = failed(error)
  }
}

/** Deleting asks first, over the credential's dialog. */
const deleteOpen = ref(false)

async function remove(): Promise<void> {
  deleteOpen.value = false
  const current = editor.value
  if (!current?.credential || current.status.kind === 'saving') {
    return
  }
  current.status = { kind: 'saving' }
  try {
    await props.deleteCredential(current.credential.id)
    current.status = { kind: 'idle' }
    current.open = false
  } catch (error) {
    current.status = failed(error)
  }
}

/** What the user has revealed, by credential: its values and the secret fields shown. */
const revealed = ref(new Map<string, { values: Record<string, string>; shown: Set<string> }>())
/** The credentials whose values are being read. */
const reading = ref(new Set<string>())

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
  if (reading.value.has(credential.id)) {
    return
  }
  reading.value = new Set(reading.value).add(credential.id)
  try {
    const values = await props.revealCredential(credential.id)
    if (values) {
      revealed.value = new Map(revealed.value).set(credential.id, { values, shown: new Set([field]) })
    }
  } finally {
    const next = new Set(reading.value)
    next.delete(credential.id)
    reading.value = next
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
  <SettingsPage title="Credentials">
    <SettingsGroup title="Your Credentials">
      <template #actions>
        <Button size="sm" @click="open(null)">New Credential…</Button>
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
                  <Tag>{{ field.kind === 'secret' ? 'Secret' : 'Visible' }}</Tag>
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
                        :loading="reading.has(credential.id)"
                        :aria-label="`${fieldValue(credential, field) === null ? 'Show' : 'Hide'} ${field.label}`"
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
                  Kept until
                  <span class="text-fg-muted">{{ credential.keepUntil ? formatMoment(credential.keepUntil) : 'Always' }}</span>
                </span>
                <span>
                  Last used
                  <span class="text-fg-muted">
                    <RelativeTime v-if="credential.lastUsedAt" :timestamp="credential.lastUsedAt" />
                    <template v-else>Never</template>
                  </span>
                </span>
              </div>
            </div>
          </template>
          <Button size="sm" @click="open(credential)">Edit</Button>
        </SettingsRow>
      </div>
      <div
        v-if="empty"
        class="select-none px-4 py-6 text-center text-[13px] text-fg-subtle"
      >
        No credentials
      </div>
    </SettingsGroup>

    <CredentialDialog
      v-if="editor"
      :is-open="editor.open"
      :overlay-store="overlayStore"
      :credential="editor.credential"
      :pending="editor.status.kind === 'saving'"
      :error="editor.status.kind === 'failed' ? editor.status : null"
      @close="editor.open = false"
      @save="save"
      @delete="deleteOpen = true"
    />
    <ConfirmDialog
      :is-open="deleteOpen"
      :overlay-store="overlayStore"
      :title="`Delete the credential “${editor?.credential?.name ?? ''}”?`"
      action="Delete"
      @close="deleteOpen = false"
      @confirm="remove"
    >
      <p>Agents can no longer read it in any conversation. This cannot be undone.</p>
    </ConfirmDialog>
  </SettingsPage>
</template>
