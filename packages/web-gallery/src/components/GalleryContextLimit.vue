<script setup lang="ts">
import { computed, ref } from 'vue'
import type { TokenUsage } from '@demicodes/protocol'
import ContextUsageIndicator from '@demicodes/web-ui/agent/ContextUsageIndicator.vue'
import ModelMenu from '@demicodes/web-ui/agent/ModelMenu.vue'
import ModelSelector from '@demicodes/web-ui/agent/ModelSelector.vue'
import { contextWindowInUse } from '@demicodes/web-ui/agent/context-limit'
import {
  applyModelChange,
  type ModelSettings,
  type ModelSettingsChange,
} from '@demicodes/web-ui/agent/model-selection'
import {
  contextLimitModels,
  contextLimitProviders,
  setGalleryContextLimit,
  withContextLimits,
} from '../fixtures/context-limits'

const props = defineProps<{
  providerId: string
  modelId: string
  /** Shows the model chip with the usage indicator; without it, the open model menu. */
  usage?: TokenUsage
}>()

const settings = ref<ModelSettings>({
  providerId: props.providerId,
  modelId: props.modelId,
  thinkingEffort: 'low',
  serviceTierId: null,
})
const models = computed(() => withContextLimits(contextLimitModels))
const selected = computed(() =>
  models.value[settings.value.providerId]?.find((model) => model.id === settings.value.modelId),
)

function change(next: ModelSettingsChange): void {
  settings.value = applyModelChange(settings.value, next)
}
</script>

<template>
  <div v-if="usage" class="flex items-center gap-1">
    <ModelSelector
      :providers="contextLimitProviders"
      :models="models"
      :settings="settings"
      @change="change"
      @context-limit="setGalleryContextLimit"
    />
    <ContextUsageIndicator
      :usage="usage"
      :context-window="contextWindowInUse(selected)"
      :input-limit="selected?.inputLimit"
    />
  </div>
  <ModelMenu
    v-else
    :providers="contextLimitProviders"
    :models="models"
    :settings="settings"
    @change="change"
    @context-limit="setGalleryContextLimit"
  />
</template>
