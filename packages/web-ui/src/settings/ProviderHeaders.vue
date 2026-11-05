<script setup lang="ts">
import { Plus, Trash2 } from '@lucide/vue'
import { nextTick, ref, watch } from 'vue'
import Button from '../ui/Button.vue'
import CommitTextInput from '../ui/CommitTextInput.vue'
import IconButton from '../ui/IconButton.vue'
import { ICON_PX } from '../ui/icon-metrics'
import InlineError from '../ui/InlineError.vue'
import Tooltip from '../ui/Tooltip.vue'
import type { SentenceText } from '../ui/ui-text'
import { t } from './locales/text'
import SettingsRow from './SettingsRow.vue'
import type { SettingsProviderHeader } from './types'

/**
 * A compatible API's Headers: the names and values every request of the entry
 * carries, as a table of rows with Add Header above it.
 *
 * Add Header adds a row with an empty name and value at once and puts the
 * focus in its name; a row without a name sends nothing. Each field commits
 * on Return or when it loses the focus, and the whole list goes to the entry
 * then. A name that is no HTTP header name or that another row has, in any
 * case, and a value with a control character, stay in their field with the
 * problem under their row until they are fixed or Escape gives the stored
 * text back.
 *
 * The rows are the view's own while the user edits them: a field stays open
 * while the entry saves, as a name's commit and the value's typing follow one
 * another, and what was committed meanwhile goes to the entry once it is free
 * (`busy`). Each row keeps a key of its own, so removing one row leaves what
 * another row's field holds, and its problem, with that row.
 *
 * The page keys it by its entry, so another entry's starts from its own list.
 */
const props = defineProps<{
  headers: readonly SettingsProviderHeader[]
  /** A change of the entry is being saved; the next waits for it. */
  busy?: boolean
}>()

const emit = defineEmits<{
  /** The entry's whole new list. */
  change: [headers: SettingsProviderHeader[]]
}>()

/** What a value says for the conversation's ID, in every language. */
const CONVERSATION_TOKEN = '{conversation}'
/** HTTP's token, which a field name is (RFC 9110 § 5.1). */
const TOKEN = /^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/
/** What a field value may not hold: a control character other than a tab (RFC 9110 § 5.5). */
const CONTROL = /[\u0000-\u0008\u000a-\u001f\u007f]/

interface Row extends SettingsProviderHeader {
  key: number
}

let nextKey = 0
function rowsOf(headers: readonly SettingsProviderHeader[]): Row[] {
  return headers.map((header) => ({ name: header.name, value: header.value, key: nextKey++ }))
}
function same(rows: readonly SettingsProviderHeader[], headers: readonly SettingsProviderHeader[]): boolean {
  return rows.length === headers.length && rows.every((row, i) => row.name === headers[i]!.name && row.value === headers[i]!.value)
}

/** The rows as the user has them, saved or about to be. */
const rows = ref<Row[]>(rowsOf(props.headers))
/** Whether the rows hold a change the entry does not have yet. */
const pending = ref(false)

/**
 * Every list the view has had: the one it opened with and each it sent. The
 * entry answers a save, and a page's state can arrive a moment late, with one
 * of them; only a list none of them is, such as another page's edit, replaces
 * the rows, and only when they hold no change of their own.
 */
const known: SettingsProviderHeader[][] = [props.headers.map(({ name, value }) => ({ name, value }))]

watch(
  () => props.headers,
  (headers) => {
    if (same(rows.value, headers)) {
      pending.value = false
    } else if (!pending.value && !known.some((list) => same(list, headers))) {
      rows.value = rowsOf(headers)
      known.push(headers.map(({ name, value }) => ({ name, value })))
    }
  },
)

/** The list last sent, which a refused save does not send again until the rows change. */
let sent: SettingsProviderHeader[] | null = null
function save(): void {
  if (props.busy || same(rows.value, props.headers) || (sent && same(rows.value, sent))) return
  sent = rows.value.map(({ name, value }) => ({ name, value }))
  known.push(sent)
  emit('change', sent)
}
watch(() => props.busy, (busy) => busy || save())

function changeRows(next: Row[]): void {
  rows.value = next
  pending.value = true
  save()
}

const table = ref<HTMLElement>()

/** What is wrong with each row's fields, by row key, while it is in its field. */
const problems = ref<Record<number, { name?: SentenceText | null; value?: SentenceText | null }>>({})

function rowProblem(key: number): SentenceText | null {
  const row = problems.value[key]
  return row?.name ?? row?.value ?? null
}

function nameProblem(index: number, name: string): SentenceText | null {
  const typed = name.trim()
  if (typed === '') return null
  if (!TOKEN.test(typed)) return t('providers.headers.invalidName')
  const taken = rows.value.some(
    (header, other) => other !== index && header.name.toLowerCase() === typed.toLowerCase(),
  )
  return taken ? t('providers.headers.takenName') : null
}

function valueProblem(value: string): SentenceText | null {
  return CONTROL.test(value) ? t('providers.headers.invalidValue') : null
}

function changed(index: number, change: Partial<SettingsProviderHeader>): void {
  changeRows(rows.value.map((row, other) => (other === index ? { ...row, ...change } : row)))
}

async function add(): Promise<void> {
  changeRows([...rows.value, { name: '', value: '', key: nextKey++ }])
  await nextTick()
  const names = table.value?.querySelectorAll<HTMLInputElement>('[data-header-name] input')
  names?.[names.length - 1]?.focus()
}

/**
 * A paste into a name of several lines, each `Name: value` or `Name=value` as a curl
 * command's or an `.env` file's are, makes a row of each in the pasted row's place;
 * a name the list has already takes the pasted value. A single line pastes as text.
 */
function paste(event: ClipboardEvent, index: number): void {
  const lines = (event.clipboardData?.getData('text') ?? '').split(/\r?\n/).map((line) => line.trim()).filter(Boolean)
  const pasted = lines.map((line) => /^(?:-H\s+)?['"]?([^:=\s'"]+)\s*[:=]\s*(.*?)['"]?$/.exec(line))
  if (lines.length < 2 || pasted.some((match) => !match)) return
  event.preventDefault()
  const kept = rows.value.map((row) => ({ ...row })).filter((row, other) => other !== index || row.name !== '' || row.value !== '')
  const added: Row[] = []
  for (const match of pasted) {
    const [, name, value] = match!
    const same = [...kept, ...added].find((row) => row.name.toLowerCase() === name!.toLowerCase())
    if (same) {
      same.value = value!
    } else {
      added.push({ name: name!, value: value!, key: nextKey++ })
    }
  }
  const at = kept.length < rows.value.length ? index : index + 1
  changeRows([...kept.slice(0, at), ...added, ...kept.slice(at)])
}

function remove(index: number): void {
  const key = rows.value[index]?.key
  if (key !== undefined) delete problems.value[key]
  changeRows(rows.value.filter((_, other) => other !== index))
}

function report(key: number, field: 'name' | 'value', problem: SentenceText | null): void {
  problems.value = { ...problems.value, [key]: { ...problems.value[key], [field]: problem } }
}
</script>

<template>
  <SettingsRow
    :label="t('providers.headers.title')"
    :description="t('providers.headers.description', { token: CONVERSATION_TOKEN })"
  >
    <Button size="sm" @click="add"
      ><Plus :size="ICON_PX.in24" /> {{ t('providers.headers.add') }}</Button
    >
    <template v-if="rows.length" #below>
      <!-- One grid for every row, so the names and the values line up: the name's
           column, the value's, and the remove button's. A phone puts a value under
           its name, the button beside the name. -->
      <div
        ref="table"
        class="grid grid-cols-[minmax(0,12rem)_minmax(0,1fr)_1.75rem] items-center gap-x-2 gap-y-2 phone:grid-cols-1 phone:gap-y-4"
      >
        <div
          v-for="(row, index) in rows"
          :key="row.key"
          class="contents phone:grid phone:grid-cols-[minmax(0,1fr)_1.75rem] phone:items-center phone:gap-2"
        >
          <span class="min-w-0" data-header-name>
            <CommitTextInput
              :model-value="row.name"
              :aria-label="t('providers.headers.nameOf', { n: index + 1 })"
              :placeholder="t('providers.headers.namePlaceholder')"
              :validate="(name) => nameProblem(index, name)"
              literal
              trim
              class="font-mono"
              @paste="paste($event, index)"
              @problem="report(row.key, 'name', $event)"
              @commit="changed(index, { name: $event })"
            />
          </span>
          <CommitTextInput
            :model-value="row.value"
            :aria-label="t('providers.headers.valueOf', { n: index + 1 })"
            :placeholder="t('providers.headers.valuePlaceholder')"
            :validate="valueProblem"
            literal
            class="font-mono phone:col-span-2 phone:row-start-2"
            @problem="report(row.key, 'value', $event)"
            @commit="changed(index, { value: $event })"
          />
          <span class="flex justify-center phone:col-start-2 phone:row-start-1">
            <Tooltip :content="t('providers.headers.remove')"
              ><IconButton
                size="sm"
                :icon="Trash2"
                variant="danger"
                :aria-label="t('providers.headers.removeOf', { n: index + 1 })"
                @click="remove(index)"
            /></Tooltip>
          </span>
          <!-- The row's problem, under the row, across the table; the rows below move down. -->
          <InlineError
            v-if="rowProblem(row.key)"
            :message="rowProblem(row.key)!"
            class="col-span-3 -mt-1 phone:col-span-2 phone:row-start-3"
          />
        </div>
      </div>
    </template>
  </SettingsRow>
</template>
