<script setup lang="ts">
import { computed } from 'vue'
import type { ThinkingConfig } from '@demicodes/core'
import { TriangleAlert, Zap } from '@lucide/vue'
import type { ModelInfo, ProviderInfo } from '../transport/protocol'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { t } from '../infra/i18n'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import IndeterminateSpinner from '../ui/IndeterminateSpinner.vue'
import Button from '../ui/Button.vue'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import { isFastMode } from './fast-mode'
import { composerModel } from './model-selection'
import { buildReasoningState, reasoningOptionLabel } from './reasoning'
import ModelMenu from './ModelMenu.vue'

const props = defineProps<{
  load?: 'loading' | 'ready' | 'failed'
  providers: ProviderInfo[]
  models: Record<string, ModelInfo[]>
  selectedProviderId?: string | null
  selectedModelId?: string | null
  thinkingConfig?: ThinkingConfig
  serviceTierId?: string | null
}>()

const emit = defineEmits<{
  retry: []
  selectModel: [providerId: string, modelId: string]
  changeThinking: [config: ThinkingConfig]
  changeServiceTier: [serviceTierId: string | null]
}>()

const state = computed(() =>
  composerModel(
    props.providers,
    props.models,
    props.selectedProviderId,
    props.selectedModelId,
  ),
)
const unavailable = computed(() => state.value.kind === 'unavailable')
const selected = computed(() =>
  state.value.kind === 'ready' ? state.value.selected : null,
)
const fast = computed(() =>
  isFastMode(selected.value?.model, props.serviceTierId),
)
// The chip names the reasoning level beside the model, quieter than the name, so the
// current effort is visible without opening the menu.
const reasoningLabel = computed(() => {
  const reasoning = buildReasoningState(selected.value?.model ?? null)
  return reasoning ? reasoningOptionLabel(reasoning, props.thinkingConfig) : ''
})
</script>

<template>
  <span
    v-if="load === 'loading'"
    class="inline-flex h-7 items-center gap-1.5 px-2 text-chrome text-fg-subtle"
    role="status"
    ><IndeterminateSpinner :size="14" /> Loading models…</span
  >
  <Button v-else-if="load === 'failed'" size="sm" @click="emit('retry')"
    >Retry models</Button
  >
  <Dropdown
    v-else-if="state.kind === 'ready' || state.kind === 'unavailable'"
    :overlay-store="appOverlayStore"
    variant="ghost"
    trigger-label="Model"
    placement="bottom-end"
  >
    <template #trigger="{ isOpen }">
      <span class="inline-flex min-w-0 items-center gap-1">
        <!-- The name sits a step above the chip's tone and the level a step below it, so the
             two stay two steps apart whether the chip is subtle (closed) or body (open). -->
        <span
          class="truncate"
          :class="isOpen ? 'text-fg-body' : 'text-fg-muted'"
          >{{ state.label }}</span
        >
        <span
          v-if="reasoningLabel"
          class="shrink-0"
          :class="isOpen ? 'text-fg-subtle' : 'text-fg-faint'"
          >{{ reasoningLabel }}</span
        >
        <Zap v-if="fast" :size="ICON_PX.in28" class="shrink-0" />
        <Tooltip v-if="unavailable" :content="t('agent.input.switchModel')">
          <TriangleAlert
            :size="ICON_PX.in28"
            class="shrink-0 text-on-warning"
          />
        </Tooltip>
      </span>
    </template>
    <template #content>
      <ModelMenu
        :providers="providers"
        :models="models"
        :selected-provider-id="selectedProviderId"
        :selected-model-id="selectedModelId"
        :thinking-config="thinkingConfig"
        :service-tier-id="serviceTierId"
        @select-model="
          (providerId, modelId) => emit('selectModel', providerId, modelId)
        "
        @change-thinking="(config) => emit('changeThinking', config)"
        @change-service-tier="(tierId) => emit('changeServiceTier', tierId)"
      />
    </template>
  </Dropdown>
</template>
