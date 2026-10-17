<script setup lang="ts">
import { t } from './locales/text'
import { t as shared } from '../locales/text'
import { computed } from 'vue'
import { CircleX, TriangleAlert, Zap } from '@lucide/vue'
import type { ModelInfo, ProviderInfo } from '../transport/protocol'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { useIconPx } from '@demicodes/web-ui/ui/icon-metrics'
import IndeterminateSpinner from '../ui/IndeterminateSpinner.vue'
import Button from '../ui/Button.vue'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import { isFastMode } from './fast-mode'
import { composerModel, type ModelSettings, type ModelSettingsChange } from './model-selection'
import type { ContextLimitChange } from './context-limit'
import { buildReasoningState, reasoningOptionLabel } from './reasoning'
import ModelMenu from './ModelMenu.vue'

const props = defineProps<{
  load?: 'loading' | 'ready' | 'failed'
  providers: ProviderInfo[]
  models: Record<string, ModelInfo[]>
  /** The conversation's model settings; none while nothing is chosen. */
  settings?: ModelSettings | null
}>()

const emit = defineEmits<{
  retry: []
  /** One change of the model settings, naming only the parts it changes. */
  change: [change: ModelSettingsChange]
  /** The user's context limit on the selected model, for all their conversations with it. */
  contextLimit: [change: ContextLimitChange]
}>()

const state = computed(() =>
  composerModel(
    props.providers,
    props.models,
    props.settings?.providerId,
    props.settings?.modelId,
  ),
)
const icons = useIconPx()
const unavailable = computed(() => state.value.kind === 'unavailable')
const selected = computed(() =>
  state.value.kind === 'ready' ? state.value.selected : null,
)
const fast = computed(() =>
  isFastMode(selected.value?.model, props.settings?.serviceTierId),
)
// The chip names the reasoning level beside the model, quieter than the name, so the
// current effort is visible without opening the menu.
const reasoningLabel = computed(() => {
  const reasoning = buildReasoningState(selected.value?.model ?? null)
  return reasoning ? reasoningOptionLabel(reasoning, props.settings?.thinkingEffort ?? null) : ''
})

// Each state is an icon and a label.
const labelText = computed(() => {
  if (props.load === 'loading')
    return t('models.loading')
  if (props.load === 'failed')
    return t('models.loadFailed')
  return reasoningLabel.value ? `${state.value.label} · ${reasoningLabel.value}` : state.value.label
})
</script>

<template>
  <!-- Where the row has no room for the whole chip, it gives way: its text is cut short with an
       ellipsis, the model's name in the ready chip, and the level, the marks and the chevron stay. -->
  <div class="flex min-w-0">
    <span
      v-if="load === 'loading'"
      class="relative inline-flex h-chip min-w-0 items-center overflow-hidden px-2 text-chrome text-fg-subtle"
      role="status"
      :aria-label="labelText"
    >
      <IndeterminateSpinner :size="icons.chip" class="shrink-0" />
      <span class="min-w-0 truncate pl-1.5">{{ labelText }}</span>
    </span>
    <span
      v-else-if="load === 'failed'"
      class="relative inline-flex h-chip min-w-0 items-center overflow-hidden pl-2 text-chrome text-on-danger"
      role="alert"
      :aria-label="labelText"
    >
      <CircleX :size="icons.chip" class="shrink-0" />
      <span class="min-w-0 truncate pl-1.5">{{ labelText }}</span>
      <Button size="sm" class="ml-1.5 shrink-0" @click="emit('retry')">{{ shared('common.retry') }}</Button>
    </span>
    <Dropdown
      v-else-if="state.kind === 'ready' || state.kind === 'unavailable'"
      :overlay-store="appOverlayStore"
      variant="ghost"
      width="shrink"
      :trigger-label="t('models.model')"
    >
      <template #trigger="{ isOpen }">
        <span class="relative inline-flex min-w-0 items-center overflow-hidden">
          <!-- The name sits a step above the chip's tone and the level a step below it, so the
               two stay two steps apart whether the chip is subtle (closed) or body (open). -->
          <span class="inline-flex min-w-0 items-center gap-1 whitespace-nowrap">
            <span class="min-w-0 truncate" :class="isOpen ? 'text-fg-body' : 'text-fg-muted'">{{ state.label }}</span>
            <span
              v-if="reasoningLabel"
              class="shrink-0"
              :class="isOpen ? 'text-fg-subtle' : 'text-fg-faint'"
            >{{ reasoningLabel }}</span>
          </span>
          <Zap v-if="fast" :size="icons.chip" class="ml-1 shrink-0" />
          <Tooltip v-if="unavailable" :content="t('models.unavailable')">
            <TriangleAlert
              :size="icons.chip"
              class="ml-1 shrink-0 text-on-warning"
            />
          </Tooltip>
        </span>
      </template>
      <template #content>
        <ModelMenu
          :providers="providers"
          :models="models"
          :settings="settings"
          @change="(change) => emit('change', change)"
          @context-limit="(change) => emit('contextLimit', change)"
        />
      </template>
    </Dropdown>
  </div>
</template>
