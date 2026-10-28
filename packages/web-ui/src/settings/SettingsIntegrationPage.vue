<script setup lang="ts">
import { t } from './locales/text'
import { computed, ref, watch } from 'vue'
import Button from '../ui/Button.vue'
import CommitTextInput from '../ui/CommitTextInput.vue'
import ConfirmDialog from '../ui/ConfirmDialog.vue'
import CopyCode from '../ui/CopyCode.vue'
import InlineError from '../ui/InlineError.vue'
import Switch from '../ui/Switch.vue'
import TextArea from '../ui/TextArea.vue'
import type { SentenceText } from '../ui/ui-text'
import { shownError } from '../infra/errors'
import type { OverlayStore } from '../overlay/overlayStore'
import type { Action } from '../ui/operations'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import {
  CAPABILITIES,
  integrationSummary,
  type IntegrationCapability,
  type IntegrationChange,
  type SettingsIntegration,
} from './integrations'

/**
 * An integration's page (`crates/backend-user-shard/src/integrations.rs`). It
 * shows:
 *
 * - its name, saved as it is committed;
 * - its token, whole with Copy only while the page holds the one its creation
 *   or Replace Token answered, hidden otherwise, since storage keeps only its
 *   hash; Replace Token asks first;
 * - a switch per capability, saved as it turns;
 * - its Clouds' init script, saved with Save, and the path of their shared
 *   folder, saved as it is committed;
 * - Delete Integration, which asks first and names what goes.
 */
const props = defineProps<{
  integration: SettingsIntegration
  /** Its token as the request that made it answered; null once that answer is gone. */
  token: string | null
  overlayStore: OverlayStore
  /** Saves a change; a refusal the user can fix is thrown, and its field shows it. */
  onChange: (change: IntegrationChange) => Promise<void>
  onReplaceToken?: Action
  /** Delete Integration, once the user confirmed it. */
  onDelete?: Action
}>()

const emit = defineEmits<{
  back: []
}>()

/** Each field's refusal, under the field, until it is committed again. */
const problems = ref<Partial<Record<keyof IntegrationChange, SentenceText>>>({})

async function save(field: keyof IntegrationChange, change: IntegrationChange): Promise<void> {
  try {
    await props.onChange(change)
    problems.value = { ...problems.value, [field]: undefined }
  } catch (error) {
    problems.value = { ...problems.value, [field]: shownError(error).message }
  }
}

function nameProblem(value: string): SentenceText | null {
  return value.trim() ? null : t('integrations.page.nameProblem')
}

// The capabilities as the switches set them: a turn of one while another's
// save is under way keeps the other's, so no save carries an older set.
const wanted = ref({ ...props.integration.capabilities })
let saving = 0
watch(
  () => props.integration.capabilities,
  (saved) => {
    if (saving === 0)
      wanted.value = { ...saved }
  },
)
async function turn(capability: IntegrationCapability, on: boolean): Promise<void> {
  wanted.value = { ...wanted.value, [capability]: on }
  saving += 1
  try {
    await save('capabilities', { capabilities: { ...wanted.value } })
  } finally {
    saving -= 1
  }
}

function pathProblem(value: string): SentenceText | null {
  const path = value.trim()
  return path.startsWith('~/') || path.startsWith('/') ? null : t('integrations.page.sharedPathProblem')
}

// The script as typed, which follows the saved one until the user edits it.
const script = ref(props.integration.initScript ?? '')
let shownScript = script.value
watch(() => props.integration.initScript, (saved) => {
  if (script.value === shownScript)
    script.value = saved ?? ''
  shownScript = saved ?? ''
})
const scriptChanged = computed(() => script.value !== (props.integration.initScript ?? ''))
const savingScript = computed(() => ({ action: 'save', target: `integration-script:${props.integration.id}` }))
const scriptButton = ref<InstanceType<typeof Button> | null>(null)
async function saveScript(): Promise<void> {
  await save('initScript', { initScript: script.value.trim() ? script.value : null })
}

const deleteOpen = ref(false)
const deleting = computed(() => ({ action: 'delete', target: `integration:${props.integration.id}` }))
async function remove(): Promise<void> {
  try {
    await props.onDelete?.()
  } finally {
    deleteOpen.value = false
  }
}

const replaceOpen = ref(false)
const replacing = computed(() => ({ action: 'replace', target: `integration-token:${props.integration.id}` }))
async function replace(): Promise<void> {
  try {
    await props.onReplaceToken?.()
  } finally {
    replaceOpen.value = false
  }
}
</script>

<template>
  <SettingsPage
    :title="integration.name"
    :back="t('integrations.title')"
    @back="emit('back')"
  >
    <template #description>
      <p>{{ integrationSummary(integration) }}</p>
    </template>
    <SettingsGroup>
      <SettingsRow :label="t('integrations.page.name')">
        <template v-if="problems.name" #detail>
          <InlineError :message="problems.name" />
        </template>
        <CommitTextInput
          :model-value="integration.name"
          :aria-label="t('integrations.page.name')"
          :validate="nameProblem"
          trim
          class="w-56 max-w-full"
          @problem="problems.name = $event ?? undefined"
          @commit="save('name', { name: $event.trim() })"
        />
      </SettingsRow>
      <SettingsRow :label="t('integrations.page.token')" :description="t('integrations.page.tokenDescription')">
        <template v-if="token" #detail>
          <div class="flex min-w-0 flex-col gap-2">
            <p class="text-[12px] leading-4 text-fg-muted">{{ t('integrations.page.tokenNew') }}</p>
            <CopyCode :code="token" />
          </div>
        </template>
        <div class="flex items-center gap-3">
          <span v-if="!token" class="text-[13px] text-fg-subtle">{{ t('integrations.page.tokenHidden') }}</span>
          <Button size="sm" :operation="replacing" @click="replaceOpen = true">{{ t('integrations.page.replace') }}</Button>
        </div>
      </SettingsRow>
    </SettingsGroup>

    <SettingsGroup
      :title="t('integrations.page.capabilities')"
      :description="t('integrations.page.capabilitiesDescription')"
    >
      <template v-if="problems.capabilities" #actions>
        <InlineError :message="problems.capabilities" />
      </template>
      <SettingsRow
        v-for="capability in CAPABILITIES"
        :key="capability"
        :label="t(`integrations.page.${capability}`)"
        :description="t(`integrations.page.${capability}Description`)"
      >
        <Switch
          :model-value="wanted[capability]"
          :aria-label="t(`integrations.page.${capability}`)"
          size="sm"
          @update:model-value="turn(capability, $event)"
        />
      </SettingsRow>
    </SettingsGroup>

    <SettingsGroup
      :title="t('integrations.page.initScript')"
      :description="t('integrations.page.initScriptDescription')"
    >
      <template #actions>
        <InlineError v-if="problems.initScript" :message="problems.initScript" />
        <Button
          ref="scriptButton"
          size="sm"
          variant="primary"
          :operation="savingScript"
          :disabled="!scriptChanged"
          @click="saveScript"
        >
          {{ t('integrations.page.save') }}
        </Button>
      </template>
      <div class="p-3">
        <TextArea
          v-model="script"
          :rows="8"
          :aria-label="t('integrations.page.initScript')"
          :placeholder="t('integrations.page.initScriptPlaceholder')"
          mono
          spellcheck="false"
          @keydown.meta.enter.prevent="scriptButton?.press()"
          @keydown.ctrl.enter.prevent="scriptButton?.press()"
        />
      </div>
    </SettingsGroup>

    <SettingsGroup>
      <SettingsRow :label="t('integrations.page.sharedPath')" :description="t('integrations.page.sharedPathDescription')">
        <template v-if="problems.sharedPath" #detail>
          <InlineError :message="problems.sharedPath" />
        </template>
        <CommitTextInput
          :model-value="integration.sharedPath"
          :aria-label="t('integrations.page.sharedPath')"
          :validate="pathProblem"
          trim
          class="w-56 max-w-full"
          @problem="problems.sharedPath = $event ?? undefined"
          @commit="save('sharedPath', { sharedPath: $event.trim() })"
        />
      </SettingsRow>
    </SettingsGroup>

    <SettingsGroup>
      <SettingsRow :label="t('integrations.page.delete')" :description="t('integrations.page.deleteDescription')">
        <Button size="sm" variant="danger" :operation="deleting" @click="deleteOpen = true">{{ t('integrations.page.deleteButton') }}</Button>
      </SettingsRow>
    </SettingsGroup>

    <ConfirmDialog
      :is-open="deleteOpen"
      :overlay-store="overlayStore"
      :title="t('integrations.page.deleteTitle', { name: integration.name })"
      :action="t('integrations.page.deleteConfirm')"
      :operation="deleting"
      :on-confirm="remove"
      @close="deleteOpen = false"
    >
      <p>{{ t('integrations.page.deleteBody', { clouds: integrationSummary(integration) }) }}</p>
    </ConfirmDialog>

    <ConfirmDialog
      :is-open="replaceOpen"
      :overlay-store="overlayStore"
      :title="t('integrations.page.replaceTitle')"
      :action="t('integrations.page.replaceConfirm')"
      :operation="replacing"
      :on-confirm="replace"
      @close="replaceOpen = false"
    >
      <p>{{ t('integrations.page.replaceBody') }}</p>
    </ConfirmDialog>
  </SettingsPage>
</template>
