<script setup lang="ts">
import { computed, ref } from 'vue'
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
import { usageOf } from '../fixtures/catalog'
import { productWould } from '../product-would'

const props = defineProps<{
  providerId: string
  modelId: string
  /**
   * The estimate of the next request, in tokens: shows the model chip with
   * the usage indicator, which counts it against the window in use; without
   * it, the open model menu.
   */
  usedTokens?: number
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

/** The usage the backend would report: the estimate against the window in use, which the limit sets. */
const usage = computed(() => {
  const window = contextWindowInUse(selected.value)
  return props.usedTokens === undefined || window === null ? null : usageOf(props.usedTokens, window)
})

function change(next: ModelSettingsChange): void {
  settings.value = applyModelChange(settings.value, next)
}
</script>

<template>
  <div v-if="usedTokens !== undefined" class="flex items-center gap-1">
    <ModelSelector
      :providers="contextLimitProviders"
      :models="models"
      :settings="settings"
      @change="change"
      @context-limit="setGalleryContextLimit"
    />
    <ContextUsageIndicator
      :usage="usage"
      @compact="productWould('Compact the conversation')"
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
