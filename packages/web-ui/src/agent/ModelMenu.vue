<script setup lang="ts">
import { computed } from 'vue'
import type { ModelInfo, ProviderInfo } from '../transport/protocol'
import { buildReasoningState, reasoningOptionIndex, reasoningOptionLabel } from './reasoning'
import { fastServiceTier, isFastMode } from './fast-mode'
import {
  contextLimitOptions,
  contextWindowInUse,
  type ContextLimitChange,
} from './context-limit'
import { formatTokens } from '../ui/token-count'
import {
  availableProviders,
  composerModel,
  modelSwitch,
  resolveSelectedModel,
  type ModelSettings,
  type ModelSettingsChange,
} from './model-selection'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuDivider from '@demicodes/web-ui/ui/MenuDivider.vue'
import MenuGroup from '@demicodes/web-ui/ui/MenuGroup.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import Switch from '@demicodes/web-ui/ui/Switch.vue'

const props = defineProps<{
  providers: ProviderInfo[]
  models: Record<string, ModelInfo[]>
  /** The conversation's model settings; none while nothing is chosen. */
  settings?: ModelSettings | null
}>()

const emit = defineEmits<{
  /** One change of the model settings, naming only the parts it changes. */
  change: [change: ModelSettingsChange]
  /** The user's context limit on the selected model, for all their conversations with it. */
  contextLimit: [change: ContextLimitChange]
}>()

const providersWithModels = computed(() => availableProviders(props.providers, props.models))
const selected = computed(
  () => resolveSelectedModel(
    props.providers,
    props.models,
    props.settings?.providerId,
    props.settings?.modelId,
  )
)
const selectedModelLabel = computed(
  () => selected.value?.model.name ?? composerModel(
    props.providers,
    props.models,
    props.settings?.providerId,
    props.settings?.modelId,
  ).label
)

const effort = computed(() => props.settings?.thinkingEffort ?? null)
const reasoningState = computed(() => buildReasoningState(selected.value?.model ?? null))
const fastTier = computed(() => fastServiceTier(selected.value?.model))
const fast = computed(() => isFastMode(selected.value?.model, props.settings?.serviceTierId))

const contextOptions = computed(() => {
  const model = selected.value?.model
  return model ? contextLimitOptions(model) : []
})
const contextLabel = computed(() => formatTokens(contextWindowInUse(selected.value?.model)))

const reasoningIndex = computed(() => {
  const state = reasoningState.value
  return state ? reasoningOptionIndex(state, effort.value) : 0
})

const reasoningLabel = computed(() => {
  const state = reasoningState.value
  return state ? reasoningOptionLabel(state, effort.value) : ''
})

function isSelectedModel(providerId: string, modelId: string): boolean {
  return selected.value?.providerId === providerId && selected.value?.modelId === modelId
}

function setFast(enabled: boolean) {
  const tier = fastTier.value
  if (!tier)
    return
  emit('change', { serviceTierId: enabled ? tier.id : null })
}

function setEffort(next: string) {
  emit('change', { thinkingEffort: next })
}

function setContextLimit(tokens: number | null) {
  const current = selected.value
  if (!current || current.model.contextLimit === tokens)
    return
  emit('contextLimit', { providerId: current.providerId, modelId: current.modelId, tokens })
}

// A switch is one change: the menu decides what of the settings the new model keeps.
function selectModel(providerId: string, model: ModelInfo) {
  if (isSelectedModel(providerId, model.id))
    return
  emit('change', modelSwitch(props.settings, selected.value?.model, providerId, model))
}
</script>

<template>
  <Menu iconless>
    <MenuItem
      v-if="fastTier"
      label="Fast Mode"
      @select="setFast(!fast)"
    >
      <template #suffix>
        <Switch
          :model-value="fast"
          size="sm"
          @click.stop
          @update:model-value="setFast"
        />
      </template>
    </MenuItem>
    <MenuItem
      v-if="reasoningState"
      label="Reasoning"
      :value="reasoningLabel"
    >
      <template #submenu>
        <Menu iconless>
          <MenuItem
            v-for="(option, index) in reasoningState.options"
            :key="option.label"
            :label="option.label"
            choice
            :is-selected="reasoningIndex === index"
            @select="setEffort(option.effort)"
          />
        </Menu>
      </template>
    </MenuItem>
    <MenuItem
      v-if="contextOptions.length"
      label="Context"
      :value="contextLabel"
    >
      <template #submenu>
        <Menu iconless>
          <MenuItem
            v-for="option in contextOptions"
            :key="option.window"
            :label="formatTokens(option.window)"
            choice
            :is-selected="(selected?.model.contextLimit ?? null) === option.tokens"
            @select="setContextLimit(option.tokens)"
          />
        </Menu>
      </template>
    </MenuItem>
    <MenuDivider v-if="fastTier || reasoningState || contextOptions.length" />
    <MenuItem label="Model" :value="selectedModelLabel">
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
              @select="selectModel(provider.id, model)"
            />
          </MenuGroup>
        </Menu>
      </template>
    </MenuItem>
  </Menu>
</template>
