<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Checkbox from '@demicodes/web-ui/ui/Checkbox.vue'
import Dialog from '@demicodes/web-ui/ui/Dialog.vue'
import Tag from '@demicodes/web-ui/ui/Tag.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import SettingsRow from './SettingsRow.vue'
import { EXTENSION_PRESETS, THINKING_EFFORTS, type SettingsModelDraft } from './types'

/**
 * One model, in its own dialog: created or edited for a custom endpoint, or viewed
 * as the catalog describes it. Tool calling is not a setting: a model without it
 * cannot drive the agent, so it is never offered.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  mode: 'create' | 'edit' | 'view'
  model: SettingsModelDraft
}>()

const emit = defineEmits<{
  close: []
  save: [model: SettingsModelDraft]
}>()

const draft = ref<SettingsModelDraft>(clone(props.model))
watch(() => props.model, (next) => { draft.value = clone(next) })

function clone(m: SettingsModelDraft): SettingsModelDraft {
  return { ...m, efforts: [...m.efforts], extensions: [...m.extensions] }
}

const editable = computed(() => props.mode !== 'view')
const title = computed(() => (props.mode === 'create' ? 'New model' : props.mode === 'edit' ? 'Edit model' : draft.value.name || draft.value.id))
const customExtension = ref('')

const cap = (word: string) => word.charAt(0).toUpperCase() + word.slice(1)

function formatTokens(n: number | null): string {
  if (n === null) return '—'
  return n >= 1_000_000 ? `${n / 1_000_000}M` : `${Math.round(n / 1000)}K`
}

function numberOrNull(value: string): number | null {
  const n = Number(value.replace(/[,\s]/g, ''))
  return value.trim() && Number.isFinite(n) ? n : null
}

function toggleEffort(effort: string) {
  const list = draft.value.efforts
  draft.value.efforts = list.includes(effort) ? list.filter((e) => e !== effort) : [...list, effort]
}

function presetState(extensions: string[]): { checked: boolean; partial: boolean } {
  const have = extensions.filter((e) => draft.value.extensions.includes(e)).length
  return { checked: have === extensions.length, partial: have > 0 && have < extensions.length }
}

function setPreset(extensions: string[], on: boolean) {
  const rest = draft.value.extensions.filter((e) => !extensions.includes(e))
  draft.value.extensions = on ? [...rest, ...extensions] : rest
}

function addExtension() {
  let value = customExtension.value.trim().toLowerCase()
  if (!value) return
  if (!value.startsWith('.')) value = `.${value}`
  if (!draft.value.extensions.includes(value)) draft.value.extensions.push(value)
  customExtension.value = ''
}

const canSave = computed(() => draft.value.id.trim().length > 0)
</script>

<template>
  <!-- Reading is a short list of facts; editing needs room for descriptions. -->
  <Dialog :is-open="isOpen" :overlay-store="overlayStore" :size="editable ? 'lg' : 'md'" :label="title" @close="emit('close')">
    <div class="flex flex-col gap-4 p-5">
      <header class="min-w-0 select-none pr-10">
        <h3 class="truncate text-[15px] font-medium text-fg-emphasis">{{ title }}</h3>
        <p v-if="mode === 'view'" class="mt-0.5 font-mono text-[12px] text-fg-subtle">{{ draft.id }}</p>
      </header>

      <div class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float">
        <SettingsRow v-if="editable" label="Model id" description="Exactly what the endpoint expects.">
          <TextInput v-model="draft.id" placeholder="model-id" class="w-64 max-w-full font-mono" :readonly="mode === 'edit'" />
        </SettingsRow>
        <SettingsRow label="Display name" :description="editable ? 'Shown in the picker instead of the id.' : undefined" :compact="!editable">
          <TextInput v-if="editable" v-model="draft.name" :placeholder="draft.id || 'Optional'" class="w-64 max-w-full" />
          <span v-else class="text-chrome text-fg">{{ draft.name || '—' }}</span>
        </SettingsRow>
        <SettingsRow label="Context window" :description="editable ? 'Tokens. Compaction triggers near this.' : undefined" :compact="!editable">
          <TextInput v-if="editable" :model-value="draft.contextWindow === null ? '' : String(draft.contextWindow)" placeholder="128000" class="w-32" @update:model-value="(v) => (draft.contextWindow = numberOrNull(v))" />
          <span v-else class="text-chrome tabular-nums text-fg">{{ formatTokens(draft.contextWindow) }}</span>
        </SettingsRow>
        <SettingsRow label="Max output" :description="editable ? 'Tokens per reply. Empty uses the endpoint default.' : undefined" :compact="!editable">
          <TextInput v-if="editable" :model-value="draft.outputLimit === null ? '' : String(draft.outputLimit)" placeholder="8192" class="w-32" @update:model-value="(v) => (draft.outputLimit = numberOrNull(v))" />
          <span v-else class="text-chrome tabular-nums text-fg">{{ formatTokens(draft.outputLimit) }}</span>
        </SettingsRow>
        <SettingsRow label="Reasoning" :description="editable ? 'Levels the composer offers. First is the default.' : undefined" :compact="!editable">
          <template v-if="editable">
            <button
              v-for="effort in THINKING_EFFORTS"
              :key="effort"
              type="button"
              class="h-6 cursor-default select-none rounded-md px-2 text-[12px] transition-colors duration-200 ease-out"
              :class="draft.efforts.includes(effort) ? 'bg-tint-accent text-on-accent' : 'bg-hover text-fg-muted hover:text-fg'"
              @click="toggleEffort(effort)"
            >
              {{ cap(effort) }}
            </button>
          </template>
          <span v-else class="text-chrome text-fg">{{ draft.efforts.length ? draft.efforts.map(cap).join(' · ') : 'None' }}</span>
        </SettingsRow>
        <SettingsRow label="Fast tier" :description="editable ? 'A service tier id the Fast switch selects.' : undefined" :compact="!editable">
          <TextInput v-if="editable" :model-value="draft.fastTier ?? ''" placeholder="Optional" class="w-32" @update:model-value="(v) => (draft.fastTier = v.trim() || null)" />
          <span v-else class="font-mono text-[12px] text-fg-muted">{{ draft.fastTier ?? '—' }}</span>
        </SettingsRow>
      </div>

      <div class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float">
        <SettingsRow label="Accepted files" :description="editable ? 'What the composer lets you attach. Text is always accepted.' : undefined" :compact="!editable">
          <span v-if="!editable && !draft.extensions.length" class="text-chrome text-fg">Text only</span>
          <span v-else-if="!editable" class="flex flex-wrap justify-end gap-1"><Tag v-for="ext in draft.extensions" :key="ext">{{ ext }}</Tag></span>
        </SettingsRow>
        <template v-if="editable">
          <SettingsRow v-for="preset in EXTENSION_PRESETS" :key="preset.id" inset :label="preset.label" :description="preset.extensions.join(' ')">
            <Checkbox
              :model-value="presetState(preset.extensions).checked"
              :partial="presetState(preset.extensions).partial"
              label=""
              @update:model-value="(on) => setPreset(preset.extensions, on)"
            />
          </SettingsRow>
          <SettingsRow inset label="Other extensions">
            <TextInput v-model="customExtension" placeholder=".heic" class="w-28" @keydown.enter="addExtension" />
            <Button size="sm" :disabled="!customExtension.trim()" @click="addExtension">Add</Button>
          </SettingsRow>
        </template>
        <div v-if="editable && draft.extensions.length" class="flex flex-wrap gap-1 px-4 py-3">
          <Tag v-for="ext in draft.extensions" :key="ext">
            {{ ext }}
            <button v-if="editable" type="button" class="ml-1 text-fg-subtle hover:text-fg" :aria-label="`Remove ${ext}`" @click="draft.extensions = draft.extensions.filter((e) => e !== ext)">×</button>
          </Tag>
        </div>
      </div>

      <div v-if="editable" class="flex justify-end gap-2">
        <Button @click="emit('close')">Cancel</Button>
        <Button variant="primary" :disabled="!canSave" @click="emit('save', clone(draft))">{{ mode === 'create' ? 'Add model' : 'Save' }}</Button>
      </div>
    </div>
  </Dialog>
</template>
