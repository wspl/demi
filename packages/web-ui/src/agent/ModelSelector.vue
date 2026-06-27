<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import type { ThinkingConfig } from '@demicodes/core'
import type { ModelInfo, ProviderInfo } from '../transport/protocol'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { buildReasoningState } from './reasoning'
import DropdownMenu from '@demicodes/web-ui/ui/DropdownMenu.vue'
import SelectorTrigger from './SelectorTrigger.vue'
import ReasoningSelector from './ReasoningSelector.vue'
import OptionMenu from '@demicodes/web-ui/ui/OptionMenu.vue'
import OptionMenuGroup from '@demicodes/web-ui/ui/OptionMenuGroup.vue'
import OptionMenuItem from '@demicodes/web-ui/ui/OptionMenuItem.vue'

const props = defineProps<{
  providers: ProviderInfo[]
  models: Record<string, ModelInfo[]>
  selectedProviderId?: string | null
  selectedModelId?: string | null
  thinkingConfig?: ThinkingConfig
}>()

const emit = defineEmits<{
  selectModel: [providerId: string, modelId: string]
  changeThinking: [config: ThinkingConfig]
}>()

interface SelectedModel {
  providerId: string
  modelId: string
}

const selectedModel = ref<SelectedModel | null>(null)

const providerModels = computed(() => props.models)

const availableProviders = computed(() =>
  props.providers.filter(p => p.isAvailable && (providerModels.value[p.id]?.length ?? 0) > 0),
)

const hasModels = computed(() =>
  availableProviders.value.some(p => (providerModels.value[p.id] ?? []).length > 0),
)

const selectedModelPreset = computed(() => {
  const selected = selectedModel.value
  if (!selected) return null
  return (providerModels.value[selected.providerId] ?? []).find(m => m.id === selected.modelId) ?? null
})

const selectedModelLabel = computed(() => selectedModelPreset.value?.name ?? '')

const reasoningState = computed(() => buildReasoningState(selectedModelPreset.value))

function isSelectedModel(providerId: string, modelId: string): boolean {
  return selectedModel.value?.providerId === providerId
    && selectedModel.value?.modelId === modelId
}

function selectModel(providerId: string, modelId: string) {
  selectedModel.value = { providerId, modelId }
  emit('selectModel', providerId, modelId)
}

function resolveFallbackSelection(): SelectedModel | null {
  for (const provider of availableProviders.value) {
    const firstModel = providerModels.value[provider.id]?.[0]
    if (firstModel) return { providerId: provider.id, modelId: firstModel.id }
  }
  return null
}

watch(
  [
    () => props.selectedProviderId,
    () => props.selectedModelId,
    providerModels,
    availableProviders,
  ],
  () => {
    if (props.selectedProviderId && props.selectedModelId) {
      selectedModel.value = { providerId: props.selectedProviderId, modelId: props.selectedModelId }
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
            :key="provider.id"
            :label="provider.label"
          >
            <OptionMenuItem
              v-for="model in providerModels[provider.id] ?? []"
              :key="`${provider.id}:${model.id}`"
              :label="model.name"
              :is-selected="isSelectedModel(provider.id, model.id)"
              @select="selectModel(provider.id, model.id); close()"
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
