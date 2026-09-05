<script setup lang="ts">
import { computed } from 'vue'
import type { ThinkingConfig } from '@demicodes/core'
import { Zap } from '@lucide/vue'
import type { ModelInfo, ProviderInfo } from '../transport/protocol'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import { isFastMode } from './fast-mode'
import { resolveSelectedModel } from './model-selection'
import ModelMenu from './ModelMenu.vue'

const props = defineProps<{
  providers: ProviderInfo[]
  models: Record<string, ModelInfo[]>
  selectedProviderId?: string | null
  selectedModelId?: string | null
  thinkingConfig?: ThinkingConfig
  serviceTierId?: string | null
}>()

const emit = defineEmits<{
  selectModel: [providerId: string, modelId: string]
  changeThinking: [config: ThinkingConfig]
  changeServiceTier: [serviceTierId: string | null]
}>()

const selected = computed(() => resolveSelectedModel(props.providers, props.models, props.selectedProviderId, props.selectedModelId))
const fast = computed(() => isFastMode(selected.value?.model, props.serviceTierId))
</script>

<template>
  <Dropdown
    v-if="selected"
    :overlay-store="appOverlayStore"
    variant="ghost"
  >
    <template #trigger>
      <span class="inline-flex min-w-0 items-center gap-1">
        <span class="truncate">{{ selected.model.name }}</span>
        <Zap v-if="fast" :size="ICON_PX.in28" class="shrink-0" />
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
        @select-model="(providerId, modelId) => emit('selectModel', providerId, modelId)"
        @change-thinking="(config) => emit('changeThinking', config)"
        @change-service-tier="(tierId) => emit('changeServiceTier', tierId)"
      />
    </template>
  </Dropdown>
</template>
