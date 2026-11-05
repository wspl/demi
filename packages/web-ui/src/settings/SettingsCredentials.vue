<script setup lang="ts">
import { t as uiText } from '../locales/text'
import { t } from './locales/text'
import { errorMessage, reportError } from '../infra/errors'
import { computed, nextTick, ref } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import { ArrowUpDown, KeyRound, Plus, Search, Trash2 } from '@lucide/vue'
import Button from '../ui/Button.vue'
import CommitTextInput from '../ui/CommitTextInput.vue'
import ConfirmDialog from '../ui/ConfirmDialog.vue'
import Dropdown from '../ui/Dropdown.vue'
import IconButton from '../ui/IconButton.vue'
import InlineError from '../ui/InlineError.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import RelativeTime from '../ui/RelativeTime.vue'
import TextInput from '../ui/TextInput.vue'
import Tooltip from '../ui/Tooltip.vue'
import TruncatedText from '../ui/TruncatedText.vue'
import { formatMoment, formatShortDay } from '../ui/dates'
import { ICON_PX } from '../ui/icon-metrics'
import { usePhone } from '../ui/phone'
import type { SentenceText } from '../ui/ui-text'
import type { CredentialFieldKind, CredentialFieldView } from '../permissions/types'
import { fieldNameFor } from './credential-fields'
import { useListSelection } from './list-selection'
import SettingsGroup from './SettingsGroup.vue'
import SettingsListItem from './SettingsListItem.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import SettingsSplit from './SettingsSplit.vue'
import type {
  SettingsCredential,
  SettingsCredentialDraft,
  SettingsCredentialKeep,
  SettingsCredentialPatch,
} from './types'

/**
 * The Credentials section: the user's credentials in a wide rail beside the
 * one selected, which is edited in place.
 *
 * ```
 * 178 CREDENTIALS     ⇅ +   Airtable sign-in                      [trash]
 * [Search          ]       Name    [Airtable sign-in              ]
 * A                        Note    [To sign in to the dashboard   ]
 * ▌Airtable sign-in        Fields                          Add Field
 *  ops                     User name  [ops                         ]
 * ```
 *
 * - The rail searches names, notes and visible values, sorts by name, use or
 *   addition, and heads its groups by letter or by age. An entry's second line
 *   is its first visible value, else its note.
 * - ⌘ and Shift select several, and a phone's Select does; the detail then
 *   deletes them together.
 * - New Credential creates one at once, kept always, with a Password field
 *   shown as missing, and opens it. Every field of the detail saves as it is
 *   left: a secret's value is written, never shown, and an emptied field is
 *   missing again. A field's kind stays as it was created, so a secret is
 *   never turned into a value the page shows.
 */
const props = defineProps<{
  credentials: SettingsCredential[]
  overlayStore: OverlayStore
  /** Creates a credential and answers its id. */
  createCredential: (draft: SettingsCredentialDraft) => Promise<string>
  /** Changes a credential; a refusal is thrown, in words for the user. */
  changeCredential: (id: string, patch: SettingsCredentialPatch) => Promise<void>
  deleteCredential: (id: string) => Promise<void>
}>()

const phone = usePhone()

type Sort = 'name' | 'recent' | 'stale' | 'added'
const SORTS: readonly Sort[] = ['name', 'recent', 'stale', 'added']
const KEEPS: readonly SettingsCredentialKeep[] = ['1h', '1d', '30d', 'always']
const sort = ref<Sort>('name')
const query = ref('')

function usedAt(credential: SettingsCredential): number {
  return credential.lastUsedAt ? Date.parse(credential.lastUsedAt) : 0
}

const ORDER: Record<Sort, (a: SettingsCredential, b: SettingsCredential) => number> = {
  name: (a, b) => a.name.localeCompare(b.name),
  recent: (a, b) => usedAt(b) - usedAt(a),
  stale: (a, b) => usedAt(a) - usedAt(b),
  added: (a, b) => Date.parse(b.createdAt) - Date.parse(a.createdAt),
}

const shown = computed(() => {
  const wanted = query.value.trim().toLowerCase()
  const found = wanted
    ? props.credentials.filter((credential) =>
        credential.name.toLowerCase().includes(wanted)
        || credential.note.toLowerCase().includes(wanted)
        || credential.fields.some((field) => field.value?.toLowerCase().includes(wanted)))
    : [...props.credentials]
  return found.sort(ORDER[sort.value])
})

const ids = computed(() => shown.value.map((credential) => credential.id))
const selection = useListSelection(ids, props.credentials.length ? [[...props.credentials].sort(ORDER.name)[0]!.id] : [])
/** A narrow page shows the detail in the rail's place once an entry opens it. */
const open = ref(false)

const chosen = computed(() => shown.value.filter((credential) => selection.selected.value.has(credential.id)))
const one = computed(() => (chosen.value.length === 1 ? chosen.value[0]! : null))

function pick(event: MouseEvent, id: string): void {
  selection.pick(event, id)
  if (!selection.checking.value && !event.metaKey && !event.ctrlKey && !event.shiftKey) {
    open.value = true
  }
}

/** An entry's second line: the account it signs in as, else what it is for. */
function subtitle(credential: SettingsCredential): string {
  return credential.fields.find((field) => field.kind === 'visible' && field.value)?.value ?? credential.note
}

const DAY = 24 * 60 * 60_000

/** The heading an entry falls under in the sort, as Contacts and Finder head their groups. */
function heading(credential: SettingsCredential, now: number): string {
  if (sort.value === 'name') {
    // A name that starts with a digit or a symbol goes under #, as in Contacts.
    const first = [...credential.name][0] ?? ''
    return /\p{L}/u.test(first) ? first.toLocaleUpperCase() : '#'
  }
  const at = sort.value === 'added' ? credential.createdAt : credential.lastUsedAt
  if (at === null) {
    return t('credentials.groups.never')
  }
  const days = (now - Date.parse(at)) / DAY
  if (days < 1) {
    return t('credentials.groups.today')
  }
  if (days < 7) {
    return t('credentials.groups.week')
  }
  if (days < 30) {
    return t('credentials.groups.month')
  }
  if (days < 365) {
    return t('credentials.groups.year')
  }
  return t('credentials.groups.older')
}

const groups = computed(() => {
  const now = Date.now()
  const out: { heading: string; items: SettingsCredential[] }[] = []
  for (const credential of shown.value) {
    const title = heading(credential, now)
    if (out.at(-1)?.heading !== title) {
      out.push({ heading: title, items: [] })
    }
    out.at(-1)!.items.push(credential)
  }
  return out
})

const countLabel = computed(() => {
  if (selection.checking.value) {
    return t('credentials.selected', { count: selection.selected.value.size })
  }
  if (query.value.trim()) {
    return t('credentials.matches', { shown: shown.value.length, count: props.credentials.length })
  }
  return t('credentials.count', { count: props.credentials.length }, props.credentials.length)
})

/** The first of `base`, `base (2)`, `base (3)`… that `taken` does not hold, whatever its case. */
function freeName(base: string, numbered: (n: number) => string, taken: readonly string[]): string {
  const names = new Set(taken.map((name) => name.toLowerCase()))
  let name = base
  for (let n = 2; names.has(name.toLowerCase()); n += 1) {
    name = numbered(n)
  }
  return name
}

const creating = ref(false)

async function create(): Promise<void> {
  creating.value = true
  try {
    const id = await props.createCredential({
      name: freeName(t('credentials.newName'), (n) => t('credentials.newNameNumbered', { n }), props.credentials.map((c) => c.name)),
      fields: [{ name: 'password', label: t('credentials.password'), kind: 'secret' }],
    })
    query.value = ''
    selection.check(false)
    selection.only(id)
    open.value = true
    await nextTick()
    document.querySelector(`[data-credential="${CSS.escape(id)}"]`)?.scrollIntoView({ block: 'nearest' })
  } catch (error) {
    reportError(t('credentials.saveFailed'), error, { site: 'credentials.createFailed', userVisible: true })
  } finally {
    creating.value = false
  }
}

/** Saves a change of the credential and answers why it was refused, said as the fix, or null once saved. */
async function change(credential: SettingsCredential, patch: SettingsCredentialPatch): Promise<SentenceText | null> {
  try {
    await props.changeCredential(credential.id, patch)
    return null
  } catch (error) {
    return errorMessage(error)
  }
}

/** A change no field shows the refusal of, such as Kept or a removed field: its failure is a toast. */
async function changeOrReport(credential: SettingsCredential, patch: SettingsCredentialPatch): Promise<void> {
  try {
    await props.changeCredential(credential.id, patch)
  } catch (error) {
    reportError(t('credentials.saveFailed'), error, { site: 'credentials.changeFailed', userVisible: true })
  }
}

/** Each field's problem, by `credentialId/field`, and the name's and the note's, while their fields keep a refused value. */
const problems = ref<Record<string, SentenceText | undefined>>({})

function views(credential: SettingsCredential): CredentialFieldView[] {
  return credential.fields.map(({ name, label, kind }) => ({ name, label, kind }))
}

function addField(credential: SettingsCredential, kind: CredentialFieldKind): void {
  const label = freeName(
    t('credentials.fieldName', { n: credential.fields.length + 1 }),
    (n) => t('credentials.fieldName', { n: credential.fields.length + n }),
    credential.fields.map((field) => field.label),
  )
  const name = fieldNameFor(label, credential.fields.map((field) => field.name))
  void changeOrReport(credential, { fields: [...views(credential), { name, label, kind }] })
}

function relabel(credential: SettingsCredential, name: string, label: string): Promise<SentenceText | null> {
  return change(credential, { fields: views(credential).map((field) => (field.name === name ? { ...field, label } : field)) })
}

function removeField(credential: SettingsCredential, name: string): void {
  void changeOrReport(credential, { fields: views(credential).filter((field) => field.name !== name) })
}

function keptLabel(credential: SettingsCredential): string {
  return credential.keepUntil
    ? t('credentials.keptUntil', { date: formatMoment(credential.keepUntil) })
    : t('credentials.keeps.always')
}

/** The credentials Delete… asks about, while it asks. */
const deleting = ref<SettingsCredential[]>([])
const deleteOpen = ref(false)

function askDelete(credentials: SettingsCredential[]): void {
  deleting.value = credentials
  deleteOpen.value = true
}

const deleteTitle = computed(() =>
  deleting.value.length === 1
    ? t('credentials.deleteTitle', { name: deleting.value[0]!.name })
    : t('credentials.deleteSeveralTitle', { count: deleting.value.length }),
)

/** Deleting, as an operation: Delete… shows it while it runs. */
const deletion = { action: 'delete', target: 'credentials:selected' }

async function remove(): Promise<void> {
  const credentials = deleting.value
  try {
    for (const credential of credentials) {
      await props.deleteCredential(credential.id)
    }
  } catch (error) {
    reportError(t('credentials.deleteFailed'), error, { site: 'credentials.deleteFailed', userVisible: true })
  } finally {
    deleteOpen.value = false
    selection.check(false)
    const first = shown.value[0]
    selection.only(first?.id ?? null)
  }
}

function several(credentials: SettingsCredential[]): string {
  const names = credentials.slice(0, 3).map((credential) => credential.name).join(', ')
  return credentials.length > 3 ? t('andMore', { names, count: credentials.length - 3 }) : names
}
</script>

<template>
  <SettingsPage wide fill :title="t('credentials.title')">
    <SettingsSplit v-model:detail-open="open" wide-list :detail-title="one?.name">
      <template #listHeader>
        <div class="flex h-8 select-none items-center gap-0.5 pl-1">
          <span class="min-w-0 truncate text-[11px] font-medium uppercase tracking-[0.04em] text-fg-subtle">{{ countLabel }}</span>
          <span class="ml-auto flex shrink-0 items-center gap-0.5">
            <template v-if="phone && credentials.length">
              <Button
                v-if="selection.checking.value"
                size="sm"
                variant="danger"
                :disabled="!chosen.length"
                @click="askDelete(chosen)"
              >{{ t('credentials.deleteShort') }}</Button>
              <Button size="sm" variant="ghost" @click="selection.check(!selection.checking.value)">
                {{ selection.checking.value ? uiText('common.done') : t('credentials.select') }}
              </Button>
            </template>
            <template v-if="!selection.checking.value">
              <Dropdown :overlay-store="overlayStore" align="end">
                <template #trigger="{ isOpen }">
                  <Tooltip :content="t('credentials.sortBy')">
                    <IconButton :icon="ArrowUpDown" size="xs" variant="ghost" :aria-label="t('credentials.sortBy')" :pressed="isOpen" />
                  </Tooltip>
                </template>
                <template #content="{ close }">
                  <Menu>
                    <MenuItem
                      v-for="option in SORTS"
                      :key="option"
                      :label="t(`credentials.sorts.${option}`)"
                      choice
                      :is-selected="sort === option"
                      @select="sort = option; close()"
                    />
                  </Menu>
                </template>
              </Dropdown>
              <Tooltip :content="t('credentials.new')">
                <IconButton :icon="Plus" size="xs" variant="ghost" :aria-label="t('credentials.new')" :loading="creating" @click="create" />
              </Tooltip>
            </template>
          </span>
        </div>
        <TextInput v-model="query" search size="sm" :placeholder="t('credentials.search')" :aria-label="t('credentials.searchLabel')">
          <template #prefix><Search :size="ICON_PX.in24" /></template>
        </TextInput>
      </template>

      <template #list>
        <template v-for="group in groups" :key="group.heading">
          <div class="sticky top-0 z-[1] flex h-7 select-none items-end bg-surface-dialog px-2 pb-1 text-[11px] font-medium text-fg-subtle">
            {{ group.heading }}
          </div>
          <SettingsListItem
            v-for="credential in group.items"
            :key="credential.id"
            :data-credential="credential.id"
            :label="credential.name"
            :detail="subtitle(credential)"
            :selected="selection.selected.value.has(credential.id)"
            :checking="selection.checking.value"
            @select="pick($event, credential.id)"
          />
        </template>
        <div v-if="!shown.length" class="select-none px-2 py-6 text-center text-[13px] text-fg-subtle">
          {{ query.trim() ? t('credentials.noMatch', { query: query.trim() }) : t('credentials.none') }}
        </div>
      </template>

      <template #detail>
        <div v-if="one" :key="one.id" class="flex flex-col gap-6">
          <SettingsGroup>
            <template #header>
              <header class="flex min-h-8 select-none items-center gap-x-1.5">
                <h3 class="min-w-0 text-[15px] font-medium text-fg-emphasis">
                  <TruncatedText :text="one.name" />
                </h3>
                <Tooltip :content="t('credentials.delete')" class="ml-auto">
                  <IconButton
                    size="sm"
                    :icon="Trash2"
                    variant="danger"
                    :aria-label="t('credentials.deleteLabel', { name: one.name })"
                    @click="askDelete([one])"
                  />
                </Tooltip>
              </header>
            </template>
            <SettingsRow :label="t('credentials.name')">
              <template v-if="problems[`${one.id}/name`]" #detail>
                <InlineError :message="problems[`${one.id}/name`]!" />
              </template>
              <CommitTextInput
                :model-value="one.name"
                :aria-label="t('credentials.name')"
                :validate="(value) => (value.trim() ? null : t('credentials.nameMissing'))"
                :save="(value) => change(one!, { name: value.trim() })"
                class="w-64 max-w-full phone:w-full"
                @problem="problems[`${one.id}/name`] = $event ?? undefined"
              />
            </SettingsRow>
            <SettingsRow :label="t('credentials.note')" :description="t('credentials.noteDescription')">
              <CommitTextInput
                :model-value="one.note"
                :aria-label="t('credentials.note')"
                :save="(value) => change(one!, { note: value })"
                class="w-64 max-w-full phone:w-full"
              />
            </SettingsRow>
          </SettingsGroup>

          <SettingsGroup :title="t('credentials.fields')">
            <template #actions>
              <Dropdown :overlay-store="overlayStore" align="end">
                <template #trigger="{ isOpen }">
                  <Button size="sm" :pressed="isOpen">{{ t('credentials.addField') }}</Button>
                </template>
                <template #content="{ close }">
                  <Menu>
                    <MenuItem :label="t('credentials.visibleField')" @select="addField(one!, 'visible'); close()" />
                    <MenuItem :label="t('credentials.secretField')" @select="addField(one!, 'secret'); close()" />
                  </Menu>
                </template>
              </Dropdown>
            </template>
            <SettingsRow
              v-for="field in one.fields"
              :key="field.name"
              class="group/field"
              :label="field.label"
              :description="field.kind === 'secret' ? t('credentials.secret') : undefined"
            >
              <template #label>
                <CommitTextInput
                  bare
                  :model-value="field.label"
                  :aria-label="t('credentials.fieldLabel')"
                  :validate="(value) => (value.trim() ? null : t('credentials.labelMissing'))"
                  :save="(value) => relabel(one!, field.name, value.trim())"
                  class="-mx-1 h-5 w-48 max-w-full rounded-sm px-1 hover:bg-hover focus-within:bg-hover"
                  @problem="problems[`${one.id}/${field.name}/label`] = $event ?? undefined"
                />
              </template>
              <template v-if="problems[`${one.id}/${field.name}/label`] || problems[`${one.id}/${field.name}`]" #detail>
                <InlineError :message="(problems[`${one.id}/${field.name}/label`] ?? problems[`${one.id}/${field.name}`])!" />
              </template>
              <CommitTextInput
                :model-value="field.value ?? ''"
                :stored="field.kind === 'secret' && !field.missing"
                :masked="field.kind === 'secret' ? 'secret' : undefined"
                :literal="field.kind === 'secret'"
                :invalid="field.missing"
                :placeholder="field.missing ? t('credentials.missing') : undefined"
                :aria-label="field.label"
                :save="(value) => change(one!, { values: { [field.name]: value } })"
                :class="one.fields.length > 1 ? 'w-56' : 'w-64'"
                class="max-w-full phone:w-full"
                @problem="problems[`${one.id}/${field.name}`] = $event ?? undefined"
              />
              <Tooltip v-if="one.fields.length > 1" :content="t('credentials.removeField')">
                <IconButton
                  size="xs"
                  :icon="Trash2"
                  variant="ghost"
                  class="invisible group-hover/field:visible group-focus-within/field:visible phone:visible"
                  :aria-label="t('credentials.removeFieldLabel', { label: field.label })"
                  @click="removeField(one!, field.name)"
                />
              </Tooltip>
            </SettingsRow>
          </SettingsGroup>

          <SettingsGroup>
            <SettingsRow :label="t('credentials.kept')">
              <Dropdown :overlay-store="overlayStore" align="end" variant="default" size="sm" :trigger-label="t('credentials.kept')">
                <template #trigger>{{ keptLabel(one) }}</template>
                <template #content="{ close }">
                  <Menu>
                    <MenuItem
                      v-for="keep in KEEPS"
                      :key="keep"
                      :label="t(`credentials.keeps.${keep}`)"
                      @select="changeOrReport(one!, { keep }); close()"
                    />
                  </Menu>
                </template>
              </Dropdown>
            </SettingsRow>
            <SettingsRow :label="t('credentials.lastUsed')">
              <span class="text-setting text-fg-muted">
                <RelativeTime v-if="one.lastUsedAt" :timestamp="one.lastUsedAt" />
                <template v-else>{{ t('credentials.never') }}</template>
              </span>
            </SettingsRow>
            <SettingsRow :label="t('credentials.added')">
              <span class="text-setting text-fg-muted">{{ formatShortDay(one.createdAt) }}</span>
            </SettingsRow>
          </SettingsGroup>
        </div>

        <div v-else-if="chosen.length > 1" class="flex h-full min-h-80 select-none flex-col items-center justify-center gap-3 text-center">
          <div class="relative size-11">
            <KeyRound class="absolute left-3 top-3 text-fg-subtle opacity-50" :size="30" />
            <KeyRound class="absolute left-0 top-0 text-fg-muted" :size="30" />
          </div>
          <div class="text-[15px] font-medium text-fg-emphasis">{{ t('credentials.several', { count: chosen.length }) }}</div>
          <div class="max-w-80 text-[13px] text-fg-subtle">{{ several(chosen) }}</div>
          <Button variant="danger" size="sm" @click="askDelete(chosen)">{{ t('credentials.deleteSeveral', { count: chosen.length }) }}</Button>
        </div>

        <div v-else class="flex h-full min-h-80 select-none items-center justify-center text-[13px] text-fg-subtle">
          {{ credentials.length ? t('credentials.noneSelected') : t('credentials.none') }}
        </div>
      </template>
    </SettingsSplit>

    <ConfirmDialog
      :is-open="deleteOpen"
      :overlay-store="overlayStore"
      :title="deleteTitle"
      :action="uiText('common.delete')"
      :goes="deleting.length > 1 ? deleting.map((credential) => credential.name) : undefined"
      :operation="deletion"
      @close="deleteOpen = false"
      @confirm="remove"
    >
      <p>{{ t('credentials.deleteBody', { count: deleting.length }, deleting.length) }}</p>
    </ConfirmDialog>
  </SettingsPage>
</template>
