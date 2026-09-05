<script setup lang="ts">
import { computed, watch } from 'vue'
import type { ThinkingConfig } from '@demicodes/core'
import type { ModelInfo, ProviderInfo } from '../transport/protocol'
import { t } from '@demicodes/web-ui/infra/i18n'
import {
  buildReasoningState,
  reasoningOptionConfig,
  reasoningOptionIndex,
  reasoningOptionLabel,
} from './reasoning'
import { fastServiceTier, isFastMode } from './fast-mode'
import { availableProviders, resolveSelectedModel } from './model-selection'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuDivider from '@demicodes/web-ui/ui/MenuDivider.vue'
import MenuGroup from '@demicodes/web-ui/ui/MenuGroup.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import Switch from '@demicodes/web-ui/ui/Switch.vue'

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

const providersWithModels = computed(() => availableProviders(props.providers, props.models))
const selected = computed(() => resolveSelectedModel(props.providers, props.models, props.selectedProviderId, props.selectedModelId))
const selectedModelLabel = computed(() => selected.value?.model.name ?? '')

const reasoningState = computed(() => buildReasoningState(selected.value?.model ?? null))
const fastTier = computed(() => fastServiceTier(selected.value?.model))
const fast = computed(() => isFastMode(selected.value?.model, props.serviceTierId))

const reasoningIndex = computed(() => {
  const state = reasoningState.value
  return state ? reasoningOptionIndex(state, props.thinkingConfig) : 0
})

const reasoningLabel = computed(() => {
  const state = reasoningState.value
  return state ? reasoningOptionLabel(state, props.thinkingConfig) : ''
})

function isSelectedModel(providerId: string, modelId: string): boolean {
  return selected.value?.providerId === providerId && selected.value?.modelId === modelId
}

function setFast(enabled: boolean) {
  const tier = fastTier.value
  if (!tier) return
  emit('changeServiceTier', enabled ? tier.id : null)
}

function setReasoningIndex(index: number) {
  const state = reasoningState.value
  if (!state) return
  emit('changeThinking', reasoningOptionConfig(state, index))
}

function selectModel(providerId: string, modelId: string) {
  const wasFast = fast.value
  emit('selectModel', providerId, modelId)
  // The switch clears the tier; keep Fast Mode on when the new model has a Fast tier of its own.
  const nextFast = fastServiceTier((props.models[providerId] ?? []).find((model) => model.id === modelId))
  if (wasFast && nextFast) emit('changeServiceTier', nextFast.id)
}

// A persisted "disabled" config on a model that cannot disable thinking is coerced to the
// model's default on mount as well as on later changes, so the chip and the session agree.
watch(
  () => [reasoningState.value, props.thinkingConfig] as const,
  ([state, config]) => {
    if (!state || state.canDisable) return
    if (config?.type === 'disabled') emit('changeThinking', state.defaultConfig)
  },
  { immediate: true },
)
</script>

<template>
  <Menu iconless>
    <MenuItem v-if="fastTier" :label="t('providers.fastMode')" @select="setFast(!fast)">
      <template #suffix>
        <Switch :model-value="fast" size="sm" @click.stop @update:model-value="setFast" />
      </template>
    </MenuItem>
    <MenuItem v-if="reasoningState" :label="t('providers.reasoning')" :value="reasoningLabel">
      <template #submenu>
        <Menu iconless>
          <MenuItem
            v-for="(option, index) in reasoningState.options"
            :key="option.label"
            :label="option.label"
            choice
            :is-selected="reasoningIndex === index"
            @select="setReasoningIndex(index)"
          />
        </Menu>
      </template>
    </MenuItem>
    <MenuDivider v-if="fastTier || reasoningState" />
    <MenuItem :label="t('providers.model')" :value="selectedModelLabel">
      <template #submenu>
        <Menu iconless>
          <MenuGroup
            v-for="provider in providersWithModels"
            :key="provider.id"
            :label="provider.label"
          >
            <MenuItem
              v-for="model in models[provider.id] ?? []"
              :key="`${provider.id}:${model.id}`"
              :label="model.name"
              choice
              :is-selected="isSelectedModel(provider.id, model.id)"
              @select="selectModel(provider.id, model.id)"
            />
          </MenuGroup>
        </Menu>
      </template>
    </MenuItem>
  </Menu>
</template>
