<script setup lang="ts">
import { computed, watchEffect } from 'vue'
import type { ThinkingConfig } from '@demicodes/core'
import type { ReasoningState } from './reasoning'
import { BrainLine } from '@mingcute/vue/brain'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { t } from '@demicodes/web-ui/infra/i18n'
import Switch from '@demicodes/web-ui/ui/Switch.vue'
import DropdownMenu from '@demicodes/web-ui/ui/DropdownMenu.vue'
import SelectorTrigger from './SelectorTrigger.vue'
import OptionMenu from '@demicodes/web-ui/ui/OptionMenu.vue'
import OptionMenuGroup from '@demicodes/web-ui/ui/OptionMenuGroup.vue'
import OptionMenuItem from '@demicodes/web-ui/ui/OptionMenuItem.vue'

const props = defineProps<{
  reasoningState: ReasoningState
  config?: ThinkingConfig
}>()

const emit = defineEmits<{
  change: [config: ThinkingConfig]
}>()

const resolvedConfig = computed(() => {
  const cfg = props.config ?? props.reasoningState.defaultConfig
  // Models that can't disable thinking (e.g. Claude Code) have no "disabled" state — a stale or
  // carried-over disabled config resolves to the default effort so the label and the request match.
  if (!props.reasoningState.canDisable && cfg.type === 'disabled') return props.reasoningState.defaultConfig
  return cfg
})

// Persist that coercion upward so the next request actually sends an effort, not a no-op "disabled".
watchEffect(() => {
  if (!props.reasoningState.canDisable && props.config?.type === 'disabled') {
    emit('change', props.reasoningState.defaultConfig)
  }
})

const isThinkingEnabled = computed(() => resolvedConfig.value.type !== 'disabled')

function toggleThinking(enabled: boolean) {
  emit('change', enabled ? props.reasoningState.defaultConfig : { type: 'disabled' })
}

function isSelected(optionConfig: ThinkingConfig): boolean {
  const cfg = resolvedConfig.value
  if (cfg.type !== optionConfig.type) return false
  if (cfg.type === 'adaptive' && optionConfig.type === 'adaptive') return cfg.effort === optionConfig.effort
  if (cfg.type === 'effort' && optionConfig.type === 'effort') return cfg.effort === optionConfig.effort
  return true
}

const defaultOption = computed(() => props.reasoningState.options.find((o) => isSelected(o.config)))

const currentDropdownLabel = computed(() => {
  if (defaultOption.value) return defaultOption.value.label
  const fallback = props.reasoningState.options.find((o) =>
    o.config.type === props.reasoningState.defaultConfig.type,
  )
  return fallback?.label ?? ''
})

function selectOption(config: ThinkingConfig, close: () => void) {
  emit('change', config)
  close()
}
</script>

<template>
  <Switch
    v-if="reasoningState.mode === 'toggle'"
    :model-value="isThinkingEnabled"
    :label="t('providers.model.thinking')"
    size="sm"
    @update:model-value="toggleThinking"
  />

  <DropdownMenu v-else-if="reasoningState.mode === 'dropdown'" :overlay-store="appOverlayStore">
    <template #trigger="{ isOpen }">
      <SelectorTrigger :is-open="isOpen">
        <BrainLine :size="13" class="mr-0.5" />
        {{ currentDropdownLabel }}
      </SelectorTrigger>
    </template>
    <template #content="{ close }">
      <OptionMenu>
        <OptionMenuItem
          v-for="option in reasoningState.options.filter(o => o.group === 'general')"
          :key="option.label"
          :label="option.label"
          :is-selected="isSelected(option.config)"
          @select="selectOption(option.config, close)"
        />
        <OptionMenuGroup :label="t('providers.reasoning')">
          <OptionMenuItem
            v-for="option in reasoningState.options.filter(o => o.group === 'effort')"
            :key="option.label"
            :label="option.label"
            :is-selected="isSelected(option.config)"
            @select="selectOption(option.config, close)"
          />
        </OptionMenuGroup>
      </OptionMenu>
    </template>
  </DropdownMenu>
</template>
