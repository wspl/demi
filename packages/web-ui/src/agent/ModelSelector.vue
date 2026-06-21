<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import type { ThinkingConfig } from '@demi/core'
import type { ModelInfo, ProviderInfo } from '../transport/protocol'
import { appOverlayStore } from '@demi/web-ui/overlay/appOverlay'
import { buildReasoningState } from './reasoning'
import DropdownMenu from '@demi/web-ui/ui/DropdownMenu.vue'
import SelectorTrigger from './SelectorTrigger.vue'
import ReasoningSelector from './ReasoningSelector.vue'
import OptionMenu from '@demi/web-ui/ui/OptionMenu.vue'
import OptionMenuGroup from '@demi/web-ui/ui/OptionMenuGroup.vue'
import OptionMenuItem from '@demi/web-ui/ui/OptionMenuItem.vue'

const props = defineProps<{
  providers: ProviderInfo[]
  models: Record<string, ModelInfo[]>
  selectedProviderType?: string | null
  selectedModelId?: string | null
  thinkingConfig?: ThinkingConfig
}>()

const emit = defineEmits<{
  selectModel: [providerType: string, modelId: string]
  changeThinking: [config: ThinkingConfig]
}>()

interface SelectedModel {
  providerType: string
  modelId: string
}

const selectedModel = ref<SelectedModel | null>(null)

const providerModels = computed(() => props.models)

const availableProviders = computed(() =>
  props.providers.filter(p => p.isAvailable && (providerModels.value[p.type]?.length ?? 0) > 0),
)

const hasModels = computed(() =>
  availableProviders.value.some(p => (providerModels.value[p.type] ?? []).length > 0),
)

const selectedModelPreset = computed(() => {
  const selected = selectedModel.value
  if (!selected) return null
  return (providerModels.value[selected.providerType] ?? []).find(m => m.id === selected.modelId) ?? null
})

const selectedModelLabel = computed(() => selectedModelPreset.value?.name ?? '')

const reasoningState = computed(() => buildReasoningState(selectedModelPreset.value))

function isSelectedModel(providerType: string, modelId: string): boolean {
  return selectedModel.value?.providerType === providerType
    && selectedModel.value?.modelId === modelId
}

function selectModel(providerType: string, modelId: string) {
  selectedModel.value = { providerType, modelId }
  emit('selectModel', providerType, modelId)
}

function resolveFallbackSelection(): SelectedModel | null {
  for (const provider of availableProviders.value) {
    const firstModel = providerModels.value[provider.type]?.[0]
    if (firstModel) return { providerType: provider.type, modelId: firstModel.id }
  }
  return null
}

watch(
  [
    () => props.selectedProviderType,
    () => props.selectedModelId,
    providerModels,
    availableProviders,
  ],
  () => {
    if (props.selectedProviderType && props.selectedModelId) {
      selectedModel.value = { providerType: props.selectedProviderType, modelId: props.selectedModelId }
      return
    }
    selectedModel.value = resolveFallbackSelection()
  },
  { immediate: true, deep: true },
)
</script>

<template>
  <template v-if="hasModels">
    <DropdownMenu :overlay-store="appOverlayStore">
      <template #trigger="{ isOpen }">
        <SelectorTrigger :is-open="isOpen">
          {{ selectedModelLabel }}
        </SelectorTrigger>
      </template>
      <template #content="{ close }">
        <OptionMenu>
          <OptionMenuGroup
            v-for="provider in availableProviders"
            :key="provider.type"
            :label="provider.label"
          >
            <OptionMenuItem
              v-for="model in providerModels[provider.type] ?? []"
              :key="`${provider.type}:${model.id}`"
              :label="model.name"
              :is-selected="isSelectedModel(provider.type, model.id)"
              @select="selectModel(provider.type, model.id); close()"
            />
          </OptionMenuGroup>
        </OptionMenu>
      </template>
    </DropdownMenu>
    <template v-if="reasoningState">
      <span class="h-3 w-px bg-overlay/10" />
      <ReasoningSelector
        :reasoning-state="reasoningState"
        v-bind="thinkingConfig ? { config: thinkingConfig } : {}"
        @change="emit('changeThinking', $event)"
      />
    </template>
  </template>
</template>
