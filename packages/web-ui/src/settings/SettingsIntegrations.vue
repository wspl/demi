<script setup lang="ts">
import { t } from './locales/text'
import { computed } from 'vue'
import AsyncRegion from '../ui/AsyncRegion.vue'
import Button from '../ui/Button.vue'
import type { OverlayStore } from '../overlay/overlayStore'
import type { Action } from '../ui/operations'
import SettingsGroup from './SettingsGroup.vue'
import SettingsIntegrationPage from './SettingsIntegrationPage.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import { integrationSummary, type IntegrationChange, type SettingsIntegration } from './integrations'

/**
 * Settings › Integrations (`crates/backend-user-shard/src/integrations.rs`): the
 * programs that build on Demi through its API, one row each with what it holds.
 *
 * Each row opens its own page, which `shown` names, in the list's place. Add
 * Integration creates one at once and opens its page, which shows its token
 * this once.
 */
const props = defineProps<{
  load?: 'loading' | 'ready' | 'failed'
  integrations: readonly SettingsIntegration[]
  /** The integration whose page is shown; null shows the list. */
  shown?: string | null
  /**
   * The token a page shows, by its integration: the one its creation or its
   * replacement answered, which no later read gives again.
   */
  tokens?: Readonly<Record<string, string>>
  overlayStore: OverlayStore
  /** Add Integration: creates one with the defaults; its page opens once it exists. */
  onAdd?: Action
  /** Changes an integration; a refusal the user can fix is thrown for its field to show. */
  onChange?: (id: string, change: IntegrationChange) => Promise<void>
  /** Replace Token, once the user confirmed it. */
  onReplaceToken?: Action<[id: string]>
  /** Delete Integration, once the user confirmed it; the list shows again once it is gone. */
  onDelete?: Action<[id: string]>
}>()
const emit = defineEmits<{
  retry: []
  /** Shows an integration's page, or the list again with null. */
  show: [id: string | null]
}>()

const page = computed(() =>
  props.shown ? props.integrations.find((integration) => integration.id === props.shown) ?? null : null,
)

async function changeShown(change: IntegrationChange): Promise<void> {
  if (page.value)
    await props.onChange?.(page.value.id, change)
}
</script>

<template>
  <SettingsIntegrationPage
    v-if="page"
    :key="page.id"
    :integration="page"
    :token="tokens?.[page.id] ?? null"
    :overlay-store="overlayStore"
    :on-change="changeShown"
    :on-replace-token="() => onReplaceToken?.(page!.id)"
    :on-delete="() => onDelete?.(page!.id)"
    @back="emit('show', null)"
  />
  <SettingsPage
    v-else
    :title="t('integrations.title')"
    :description="t('integrations.description')"
  >
    <SettingsGroup :title="t('integrations.yours')">
      <template #actions>
        <Button size="sm" @click="onAdd?.()">{{ t('integrations.add') }}</Button>
      </template>
      <AsyncRegion
        :state="load"
        :label="t('integrations.loading')"
        @retry="emit('retry')"
      >
        <SettingsRow
          v-for="integration in integrations"
          :key="integration.id"
          :label="integration.name"
          :description="integrationSummary(integration)"
          :target="`integration:${integration.id}`"
          navigable
          @click="emit('show', integration.id)"
        />
        <div
          v-if="!integrations.length"
          class="select-none px-4 py-6 text-center text-[13px] text-fg-subtle"
        >
          {{ t('integrations.none') }}
        </div>
      </AsyncRegion>
    </SettingsGroup>
  </SettingsPage>
</template>
