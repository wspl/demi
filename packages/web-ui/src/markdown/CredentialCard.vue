<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { Eye, EyeOff, KeyRound } from '@lucide/vue'
import { t } from '../locales/text'
import VerifyDialog from '../auth/VerifyDialog.vue'
import { useConfirmation } from '../auth/verification'
import { appOverlayStore } from '../overlay/appOverlay'
import CopyButton from '../ui/CopyButton.vue'
import IconButton from '../ui/IconButton.vue'
import Tooltip from '../ui/Tooltip.vue'
import TruncatedText from '../ui/TruncatedText.vue'
import type { SettingsCredentialField } from '../settings/types'
import type { MessageCredentials } from './types'

/**
 * A credential block's card: the saved credential a reply names, its
 * secret fields masked until the user acts.
 *
 * - One field shows in the card's second line, after its label when it is
 *   one of several; several, each on a row of its own under the name. A
 *   block that names a field shows that one.
 * - Show and Copy ask the page for a secret's value each time, which asks
 *   the user to confirm it is them when the backend says so, then asks
 *   again; Hide forgets it. Copy of a shown value copies it as it shows.
 * - A visible field shows its value, with Copy.
 * - No longer saved: no credential of the name, or none with the field.
 * - The name comes from the block: while the page reads the credentials the
 *   card shows it alone, and where the page has none to read, as on a
 *   share, or could not read them, it shows it with Saved credential.
 */
const props = defineProps<{
  name: string
  /** The field the block names; null for every field. */
  field: string | null
  credentials: MessageCredentials | undefined
}>()

watch(() => props.credentials, (credentials, _, onCleanup) => {
  if (credentials)
    onCleanup(credentials.hold())
}, { immediate: true })

const lookup = computed(() => props.credentials?.find(props.name) ?? null)
const credential = computed(() => (lookup.value?.state === 'found' ? lookup.value.credential : null))
const fields = computed<SettingsCredentialField[]>(() => {
  const all = credential.value?.fields ?? []
  return props.field === null ? all : all.filter((field) => field.name === props.field)
})
/** What the card's second line says in place of a value; null while it shows fields or still waits. */
const status = computed(() => {
  if (!lookup.value || lookup.value.state === 'failed')
    return t('markdown.credential.saved')
  if (lookup.value.state === 'missing' || (credential.value && fields.value.length === 0))
    return t('markdown.credential.missing')
  return null
})

/** The values of the secret fields shown, by field; the credential's id says whose. */
const values = ref<Map<string, string>>(new Map())
/** The secret fields shown unmasked. */
const shown = ref<Set<string>>(new Set())
watch(() => credential.value?.id, () => {
  values.value = new Map()
  shown.value = new Set()
})

const confirmation = useConfirmation()

/** A secret field's value, asked for with the user's confirmation when needed; null once there is none. */
async function valueOf(field: string): Promise<string | null> {
  const id = credential.value?.id
  if (!id || !props.credentials)
    return null
  const credentials = props.credentials
  const outcome = await confirmation.confirmed(() => credentials.reveal(id, field), t('markdown.credential.confirm', { name: props.name }))
  return outcome?.kind === 'revealed' ? outcome.value : null
}

async function made(field: SettingsCredentialField): Promise<string> {
  const value = field.value ?? values.value.get(field.name) ?? await valueOf(field.name)
  if (value === null)
    throw new Error('No value')
  return value
}

async function toggle(field: string): Promise<void> {
  if (shown.value.has(field)) {
    const next = new Set(shown.value)
    next.delete(field)
    shown.value = next
    const kept = new Map(values.value)
    kept.delete(field)
    values.value = kept
    return
  }
  const value = await valueOf(field)
  if (value === null)
    return
  values.value = new Map(values.value).set(field, value)
  shown.value = new Set(shown.value).add(field)
}

/** What a field's value shows as: a visible one's or a shown secret's text, else dots. */
function shownValue(field: SettingsCredentialField): string {
  if (field.value !== null)
    return field.value
  return shown.value.has(field.name) ? values.value.get(field.name) ?? '' : '••••••••••••'
}

const lone = computed(() => fields.value.length === 1 ? fields.value[0]! : null)
/** Whether the fields are rows under the name, which then stands on a line of its own. */
const rows = computed(() => !lone.value && status.value === null && lookup.value?.state === 'found')
/** The lone field's label, said when the credential has other fields. */
const loneLabel = computed(() => (lone.value && (credential.value?.fields.length ?? 0) > 1 ? lone.value.label : null))
</script>

<template>
  <div class="w-[360px] max-w-full select-none rounded-lg border border-line bg-surface" data-credential-card-body>
    <div class="flex items-center gap-3 pl-3 pr-2" :class="rows ? 'h-10' : 'h-14'">
      <KeyRound :size="rows ? 16 : 20" class="shrink-0 text-fg-muted" aria-hidden="true" />
      <div class="min-w-0 flex-1">
        <TruncatedText class="text-chrome text-fg" :text="name" />
        <div v-if="status !== null" class="truncate text-detail text-fg-muted">{{ status }}</div>
        <div v-else-if="lone" class="flex min-w-0 gap-2 text-detail text-fg-muted">
          <span v-if="loneLabel !== null" class="shrink-0">{{ loneLabel }}</span>
          <span
            class="min-w-0 truncate font-mono"
            :class="lone.value !== null || shown.has(lone.name) ? 'select-text' : ''"
          >{{ shownValue(lone) }}</span>
        </div>
      </div>
      <template v-if="lone && status === null">
        <Tooltip v-if="lone.value === null" :content="shown.has(lone.name) ? t('common.hide') : t('common.show')">
          <IconButton
            :icon="shown.has(lone.name) ? EyeOff : Eye"
            variant="ghost"
            :aria-label="shown.has(lone.name) ? t('common.hide') : t('common.show')"
            @click="toggle(lone.name)"
          />
        </Tooltip>
        <CopyButton :made="() => made(lone!)" size="md" />
      </template>
    </div>
    <template v-if="rows">
      <div v-for="row in fields" :key="row.name" class="flex h-9 items-center gap-3 border-t border-line pl-3 pr-2">
        <div class="w-24 shrink-0 truncate text-detail text-fg-muted">{{ row.label }}</div>
        <div
          class="min-w-0 flex-1 truncate font-mono text-detail text-fg"
          :class="row.value !== null || shown.has(row.name) ? 'select-text' : ''"
        >{{ shownValue(row) }}</div>
        <Tooltip v-if="row.value === null" :content="shown.has(row.name) ? t('common.hide') : t('common.show')">
          <IconButton
            :icon="shown.has(row.name) ? EyeOff : Eye"
            variant="ghost"
            :aria-label="shown.has(row.name) ? t('common.hide') : t('common.show')"
            @click="toggle(row.name)"
          />
        </Tooltip>
        <CopyButton :made="() => made(row)" size="md" />
      </div>
    </template>
  </div>
  <VerifyDialog
    v-if="credentials"
    :is-open="confirmation.message.value !== null"
    :overlay-store="appOverlayStore"
    :host="credentials.verification"
    :email="credentials.email"
    :message="confirmation.message.value ?? ''"
    @close="confirmation.settle(false)"
    @verified="confirmation.settle(true)"
  />
</template>
