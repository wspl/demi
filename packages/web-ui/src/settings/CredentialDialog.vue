<script setup lang="ts">
import { t as uiText } from '../locales/text'
import { t } from './locales/text'
import type { ShownError } from '../infra/errors'
import { computed, ref, watch } from 'vue'
import { Ellipsis } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import Dialog from '../ui/Dialog.vue'
import Dropdown from '../ui/Dropdown.vue'
import { formatVerbAt } from '../ui/dates'
import IconButton from '../ui/IconButton.vue'
import InlineError from '../ui/InlineError.vue'
import Menu from '../ui/Menu.vue'
import MenuDivider from '../ui/MenuDivider.vue'
import MenuItem from '../ui/MenuItem.vue'
import type { Action, Operation } from '../ui/operations'
import ScrollArea from '../ui/ScrollArea.vue'
import TextInput from '../ui/TextInput.vue'
import Tooltip from '../ui/Tooltip.vue'
import {
  draftFields,
  FIELDS_MAX,
  fieldNames,
  LABEL_MAX,
  NAME_MAX,
  textProblem,
  valueProblem,
  type EditorField,
} from './credential-fields'
import type { SettingsCredential, SettingsCredentialDraft } from './types'

/**
 * Creates a credential or edits one, asking no more than a credential is, as the
 * Passwords app's editor does: its name and its note, then one row for each
 * field.
 *
 * - A row holds the field's label and its value side by side, with the name an
 *   agent reads it by under the label and a ⋯ menu at the row's end that makes it
 *   secret or visible and deletes it.
 * - Every input starts and ends at the same edges: the cards share one grid of a
 *   label column, an input column and the menu's column.
 * - A new field is read by a name made from its label. A saved field keeps its
 *   name, a visible one shows its value, and a secret one's value left empty
 *   keeps the saved one, which the page never has.
 * - A credential kept for a while says under its note when it goes, beside Keep
 *   Always. One kept always says nothing of it.
 * - A problem shows under its input once the user has left it, and Save waits
 *   until nothing is wrong.
 *
 * The host saves (`onSave`, whose promise Save shows until it settles). A refusal
 * shows under the fields, and the draft stays.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  /** The credential edited; null for a new one. */
  credential: SettingsCredential | null
  onSave?: Action<[draft: SettingsCredentialDraft]>
  /** What Save does, and what Delete does, one group on the credential (`operations.ts`): while one works, the other cannot be pressed. */
  saveOperation?: Operation
  deleteOperation?: Operation
  /** Why the last save failed, with the request it came from. */
  error?: ShownError | null
}>()

const emit = defineEmits<{
  close: []
  delete: []
}>()

let keys = 0
function newField(): EditorField {
  keys += 1
  return { key: keys, saved: null, label: '', kind: 'secret', value: '' }
}

const name = ref('')
const note = ref('')
/** Whether the user chose Keep Always for a credential kept for a while; a new one is kept always. */
const keepAlways = ref(false)
const fields = ref<EditorField[]>([])
/** The inputs the user has left, by `name`, `label:<key>` or `value:<key>`: a problem shows under them. */
const touched = ref(new Set<string>())

function reset(): void {
  const credential = props.credential
  name.value = credential?.name ?? ''
  note.value = credential?.note ?? ''
  keepAlways.value = false
  fields.value = credential
    ? credential.fields.map((field) => {
        keys += 1
        // A visible field shows its value, as the list does; a secret's is never on the page.
        return { key: keys, saved: field.name, label: field.label, kind: field.kind, value: field.value ?? '' }
      })
    : [newField()]
  touched.value = new Set()
}
reset()

watch(
  () => props.isOpen,
  (open) => {
    if (open) {
      reset()
    }
  },
)

function touch(id: string): void {
  touched.value = new Set(touched.value).add(id)
}

const namesRead = computed(() => fieldNames(fields.value))
const nameProblem = computed(() => textProblem(name.value, NAME_MAX))
const labelProblems = computed(() => fields.value.map((field) => textProblem(field.label, LABEL_MAX)))
const valueProblems = computed(() => fields.value.map((field) => valueProblem(field.value, field.saved !== null)))
const canSave = computed(
  () =>
    nameProblem.value === null &&
    fields.value.length > 0 &&
    labelProblems.value.every((problem) => problem === null) &&
    valueProblems.value.every((problem) => problem === null),
)

function shown(id: string, problem: string | null): string | null {
  return touched.value.has(id) ? problem : null
}

function addField(): void {
  fields.value.push(newField())
}

function removeField(key: number): void {
  fields.value = fields.value.filter((field) => field.key !== key)
}

/** When the saved credential goes, "Removed today at 6:00 PM"; null for one kept always, or once the user chose Keep Always. */
const removedAt = computed(() => {
  const until = props.credential?.keepUntil
  return until && !keepAlways.value ? formatVerbAt('Removed', until, new Date()) : null
})

function save(): unknown {
  if (!canSave.value) {
    return
  }
  return props.onSave?.({
    name: name.value.trim(),
    note: note.value.trim(),
    keep: props.credential === null || keepAlways.value ? 'always' : null,
    fields: draftFields(fields.value),
  })
}

const title = computed(() => (props.credential ? t('credentialDialog.edit') : t('credentialDialog.new')))

/**
 * The cards' one grid: the label column, the inputs' column and the menu's
 * column, which the top card leaves empty so its inputs end where the fields' do.
 *
 * A phone stacks a row as a settings row drops its control under its label: the
 * label above its input, and a field's label input above its value, with its menu
 * beside the label input. Every other input spans the card.
 */
const GRID = 'grid grid-cols-[8.5rem_minmax(0,1fr)_1.75rem] items-start gap-x-3 phone:grid-cols-1'
</script>

<template>
  <Dialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    size="lg"
    :label="title"
    @close="emit('close')"
  >
    <div class="flex min-h-0 flex-col">
      <header class="min-w-0 select-none p-5 pb-4 pr-10">
        <h3 class="text-[15px] font-medium text-fg-emphasis">{{ title }}</h3>
      </header>

      <!-- Only the fields scroll; the title and the buttons stay in place. -->
      <ScrollArea class="min-h-0" viewport-class="flex flex-col gap-3 px-5">
        <div class="gap-y-3 rounded-xl border border-line bg-surface-float p-4" :class="GRID">
          <label class="contents">
            <span class="flex h-field select-none items-center text-setting text-fg phone:h-auto">{{ t('credentialDialog.name') }}</span>
            <span class="flex min-w-0 flex-col gap-1">
              <TextInput
                v-model="name"
                :focused="!credential"
                :aria-label="t('credentialDialog.name')"
                @blur="touch('name')"
              />
              <InlineError v-if="shown('name', nameProblem)" :message="shown('name', nameProblem)!" />
            </span>
            <span class="phone:hidden" />
          </label>
          <label class="contents">
            <span class="flex h-field select-none items-center text-setting text-fg phone:h-auto">{{ t('credentialDialog.note') }}</span>
            <span class="min-w-0">
              <TextInput
                v-model="note"
                :placeholder="t('credentialDialog.notePlaceholder')"
                :aria-label="t('credentialDialog.note')"
              />
            </span>
            <span class="phone:hidden" />
          </label>
          <template v-if="removedAt">
            <span class="phone:hidden" />
            <div class="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1">
              <span class="select-none text-setting-small text-fg-subtle">{{ removedAt }}</span>
              <Button size="sm" @click="keepAlways = true">{{ t('credentialDialog.keepAlways') }}</Button>
            </div>
          </template>
        </div>

        <div class="flex min-h-7 items-center justify-between gap-3 pt-1">
          <h4 class="select-none text-[13px] font-medium text-fg-emphasis">{{ t('credentialDialog.fields') }}</h4>
          <Button
            size="sm"
            :disabled="fields.length >= FIELDS_MAX"
            :disabled-reason="t('credentialDialog.fieldsMax', { max: FIELDS_MAX })"
            @click="addField"
          >{{ t('credentialDialog.addField') }}</Button>
        </div>
        <div class="gap-y-4 rounded-xl border border-line bg-surface-float p-4" :class="GRID">
          <!-- One field: three cells of the card's grid, or on a phone a grid of its own, its value under its label and menu. -->
          <div
            v-for="(field, index) in fields"
            :key="field.key"
            class="contents phone:grid phone:grid-cols-[minmax(0,1fr)_auto] phone:gap-x-2 phone:gap-y-2"
          >
            <div class="flex min-w-0 flex-col gap-1">
              <TextInput
                v-model="field.label"
                :placeholder="t('credentialDialog.labelPlaceholder')"
                :aria-label="t('credentialDialog.labelOf', { n: index + 1 })"
                @blur="touch(`label:${field.key}`)"
              />
              <span
                v-if="field.saved !== null || field.label.trim() !== ''"
                class="min-w-0 truncate px-2.5 font-mono text-detail text-fg-subtle"
              >{{ namesRead[index] }}</span>
              <InlineError
                v-if="shown(`label:${field.key}`, labelProblems[index]!)"
                :message="shown(`label:${field.key}`, labelProblems[index]!)!"
              />
            </div>
            <div class="flex min-w-0 flex-col gap-1 phone:col-span-2 phone:row-start-2">
              <TextInput
                v-model="field.value"
                :masked="field.kind === 'secret' ? 'secret' : undefined"
                literal
                :placeholder="field.saved !== null ? t('credentialDialog.keepSaved') : undefined"
                :aria-label="t('credentialDialog.valueOf', { n: index + 1 })"
                @blur="touch(`value:${field.key}`)"
              />
              <InlineError
                v-if="shown(`value:${field.key}`, valueProblems[index]!)"
                :message="shown(`value:${field.key}`, valueProblems[index]!)!"
              />
            </div>
            <div class="flex h-field items-center justify-center phone:col-start-2 phone:row-start-1">
              <Dropdown :overlay-store="overlayStore">
                <template #trigger="{ isOpen: menuOpen }">
                  <Tooltip :content="t('credentialDialog.actions')">
                    <IconButton
                      :icon="Ellipsis"
                      size="sm"
                      variant="ghost"
                      :aria-label="t('credentialDialog.fieldActions', { n: index + 1 })"
                      :pressed="menuOpen"
                    />
                  </Tooltip>
                </template>
                <template #content="{ close }">
                  <Menu>
                    <MenuItem
                      :label="t('credentials.secret')"
                      choice
                      :is-selected="field.kind === 'secret'"
                      @select="field.kind = 'secret'; close()"
                    />
                    <MenuItem
                      :label="t('credentials.visible')"
                      choice
                      :is-selected="field.kind === 'visible'"
                      @select="field.kind = 'visible'; close()"
                    />
                    <MenuDivider />
                    <MenuItem
                      :label="t('credentialDialog.deleteField')"
                      is-danger
                      :disabled="fields.length === 1"
                      :disabled-reason="t('credentialDialog.needsField')"
                      @select="removeField(field.key)"
                    />
                  </Menu>
                </template>
              </Dropdown>
            </div>
          </div>
        </div>
      </ScrollArea>
      <InlineError v-if="error" class="px-5 pt-3" :message="error.message" :request-id="error.requestId" />
    </div>
    <template v-if="credential" #footer-leading>
      <Button variant="danger" :operation="deleteOperation" @click="emit('delete')">{{ uiText('common.delete') }}</Button>
    </template>
    <template #footer>
      <Button @click="emit('close')">{{ uiText('common.cancel') }}</Button>
      <Button variant="primary" :disabled="!canSave" :operation="saveOperation" @click="save">
        {{ credential ? uiText('common.save') : t('credentialDialog.create') }}
      </Button>
    </template>
  </Dialog>
</template>
