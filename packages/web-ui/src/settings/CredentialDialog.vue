<script setup lang="ts">
import type { ShownError } from '../infra/errors'
import { computed, ref, watch } from 'vue'
import { Trash2 } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import { KEEP_OPTIONS, type CredentialKeep } from '../permissions/types'
import Button from '../ui/Button.vue'
import Dialog from '../ui/Dialog.vue'
import Dropdown from '../ui/Dropdown.vue'
import { formatMoment } from '../ui/dates'
import IconButton from '../ui/IconButton.vue'
import InlineError from '../ui/InlineError.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import type { Action, Operation } from '../ui/operations'
import ScrollArea from '../ui/ScrollArea.vue'
import Segmented from '../ui/Segmented.vue'
import TextInput from '../ui/TextInput.vue'
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
import SettingsRow from './SettingsRow.vue'
import type { SettingsCredential, SettingsCredentialDraft } from './types'

/**
 * Creates a credential or edits one (`credentials.md` § A credential): its
 * name, its note, how long to keep it, and its fields, each with a label, a
 * kind and a value. A new field is read by a name made from its label, shown
 * under it; a saved field keeps its name, and its value left empty keeps the
 * saved one, which the page never has. A problem shows under its field once
 * the user has left it, and Save waits until nothing is wrong. The host
 * saves (`onSave`, whose promise Save shows until it settles); a refusal shows
 * under the fields, and the draft stays.
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

const KINDS = [
  { value: 'secret', label: 'Secret' },
  { value: 'visible', label: 'Visible' },
] as const

let keys = 0
function newField(): EditorField {
  keys += 1
  return { key: keys, saved: null, label: '', kind: 'secret', value: '' }
}

const name = ref('')
const note = ref('')
/** The keep the user chose; null while a saved credential's own time stands. */
const keep = ref<CredentialKeep | null>(null)
const fields = ref<EditorField[]>([])
/** The inputs the user has left, by `name`, `label:<key>` or `value:<key>`: a problem shows under them. */
const touched = ref(new Set<string>())

function reset(): void {
  const credential = props.credential
  name.value = credential?.name ?? ''
  note.value = credential?.note ?? ''
  keep.value = credential ? null : 'always'
  fields.value = credential
    ? credential.fields.map((field) => {
        keys += 1
        return { key: keys, saved: field.name, label: field.label, kind: field.kind, value: '' }
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

function save(): unknown {
  if (!canSave.value) {
    return
  }
  return props.onSave?.({
    name: name.value.trim(),
    note: note.value.trim(),
    keep: keep.value,
    fields: draftFields(fields.value),
  })
}

const title = computed(() => (props.credential ? 'Edit Credential' : 'New Credential'))
const keepLabel = computed(() => {
  if (keep.value !== null) {
    return KEEP_OPTIONS.find((option) => option.value === keep.value)!.label
  }
  const until = props.credential?.keepUntil
  return until ? formatMoment(until) : 'Always'
})
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
        <div class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float">
          <SettingsRow label="Name">
            <template v-if="shown('name', nameProblem)" #detail>
              <InlineError :message="shown('name', nameProblem)!" />
            </template>
            <TextInput
              v-model="name"
              :focused="!credential"
              class="w-64 max-w-full"
              aria-label="Name"
              @blur="touch('name')"
            />
          </SettingsRow>
          <SettingsRow label="Note" description="Optional. Why an agent needs it.">
            <TextInput
              v-model="note"
              class="w-80 max-w-full"
              aria-label="Note"
            />
          </SettingsRow>
          <SettingsRow label="Keep until">
            <Dropdown
              :overlay-store="overlayStore"
              size="sm"
              variant="default"
              trigger-label="Keep until"
            >
              <template #trigger>{{ keepLabel }}</template>
              <template #content="{ close }">
                <Menu>
                  <MenuItem
                    v-for="option in KEEP_OPTIONS"
                    :key="option.value"
                    :label="option.label"
                    choice
                    :is-selected="keep === option.value"
                    @select="keep = option.value; close()"
                  />
                </Menu>
              </template>
            </Dropdown>
          </SettingsRow>
        </div>

        <div class="flex min-h-7 items-center justify-between gap-3 pt-1">
          <h4 class="select-none text-[13px] font-medium text-fg-emphasis">Fields</h4>
          <Button
            size="sm"
            :disabled="fields.length >= FIELDS_MAX"
            :disabled-reason="`A credential has at most ${FIELDS_MAX} fields.`"
            @click="addField"
          >Add Field</Button>
        </div>
        <div
          v-for="(field, index) in fields"
          :key="field.key"
          class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float"
        >
          <SettingsRow label="Label">
            <template v-if="field.saved !== null || field.label.trim() !== ''" #description>
              Read as <span class="font-mono">{{ namesRead[index] }}</span>
            </template>
            <template v-if="shown(`label:${field.key}`, labelProblems[index]!)" #detail>
              <InlineError :message="shown(`label:${field.key}`, labelProblems[index]!)!" />
            </template>
            <TextInput
              v-model="field.label"
              class="w-56 max-w-full"
              placeholder="API key"
              :aria-label="`Label of field ${index + 1}`"
              @blur="touch(`label:${field.key}`)"
            />
            <IconButton
              :icon="Trash2"
              size="sm"
              variant="ghost"
              :aria-label="`Remove field ${index + 1}`"
              :disabled="fields.length === 1"
              disabled-reason="A credential needs a field."
              @click="removeField(field.key)"
            />
          </SettingsRow>
          <SettingsRow label="Kind">
            <Segmented v-model="field.kind" :options="KINDS" size="sm" />
          </SettingsRow>
          <SettingsRow label="Value">
            <template v-if="shown(`value:${field.key}`, valueProblems[index]!)" #detail>
              <InlineError :message="shown(`value:${field.key}`, valueProblems[index]!)!" />
            </template>
            <TextInput
              v-model="field.value"
              :masked="field.kind === 'secret' ? 'secret' : undefined"
              literal
              class="w-80 max-w-full"
              :placeholder="field.saved !== null ? 'Leave empty to keep it' : undefined"
              :aria-label="`Value of field ${index + 1}`"
              @blur="touch(`value:${field.key}`)"
            />
          </SettingsRow>
        </div>
      </ScrollArea>
      <InlineError v-if="error" class="px-5 pt-3" :message="error.message" :request-id="error.requestId" />
    </div>
    <template v-if="credential" #footer-leading>
      <Button variant="danger" :operation="deleteOperation" @click="emit('delete')">Delete</Button>
    </template>
    <template #footer>
      <Button @click="emit('close')">Cancel</Button>
      <Button variant="primary" :disabled="!canSave" :operation="saveOperation" @click="save">
        {{ credential ? 'Save' : 'Create Credential' }}
      </Button>
    </template>
  </Dialog>
</template>
