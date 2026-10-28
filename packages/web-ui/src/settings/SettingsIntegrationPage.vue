<script setup lang="ts">
import { t } from './locales/text'
import { computed, ref, watch } from 'vue'
import Button from '../ui/Button.vue'
import CommitTextInput from '../ui/CommitTextInput.vue'
import ConfirmDialog from '../ui/ConfirmDialog.vue'
import CopyCode from '../ui/CopyCode.vue'
import InlineError from '../ui/InlineError.vue'
import Switch from '../ui/Switch.vue'
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
 * - a switch per capability, saved as it turns.
 */
const props = defineProps<{
  integration: SettingsIntegration
  /** Its token as the request that made it answered; null once that answer is gone. */
  token: string | null
  overlayStore: OverlayStore
  /** Saves a change; a refusal the user can fix is thrown, and its field shows it. */
  onChange: (change: IntegrationChange) => Promise<void>
  onReplaceToken?: Action
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
