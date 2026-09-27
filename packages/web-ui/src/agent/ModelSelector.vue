<script setup lang="ts">
import { computed, ref } from 'vue'
import { CircleX, Sparkles, TriangleAlert, Zap } from '@lucide/vue'
import type { ModelInfo, ProviderInfo } from '../transport/protocol'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import { COMPACT_LABEL_CLASS, useRoomLabel } from '../ui/label-room'
import IndeterminateSpinner from '../ui/IndeterminateSpinner.vue'
import Button from '../ui/Button.vue'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import { isFastMode } from './fast-mode'
import { composerModel, type ModelSettings, type ModelSettingsChange } from './model-selection'
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
}>()

const state = computed(() =>
  composerModel(
    props.providers,
    props.models,
    props.settings?.providerId,
    props.settings?.modelId,
  ),
)
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

// Each state is an icon and a label. The label shows while the editor keeps its
// least width with it; otherwise the selector is its icon, and the label its tooltip.
const labelText = computed(() => {
  if (props.load === 'loading')
    return 'Loading models…'
  if (props.load === 'failed')
    return "Couldn't load models."
  return reasoningLabel.value ? `${state.value.label} · ${reasoningLabel.value}` : state.value.label
})
const label = ref<HTMLElement | null>(null)
const compact = useRoomLabel(label, () => labelText.value)
</script>

<template>
  <Tooltip :content="labelText" :disabled="!compact" tag="div" class="flex">
    <span
      v-if="load === 'loading'"
      class="relative inline-flex h-7 items-center overflow-hidden px-2 text-chrome text-fg-subtle"
      role="status"
      :aria-label="labelText"
    >
      <IndeterminateSpinner :size="14" />
      <span ref="label" class="whitespace-nowrap pl-1.5" :class="compact ? COMPACT_LABEL_CLASS : ''">{{ labelText }}</span>
    </span>
    <span
      v-else-if="load === 'failed'"
      class="relative inline-flex h-7 items-center overflow-hidden pl-2 text-chrome text-on-danger"
      role="alert"
      :aria-label="labelText"
    >
      <CircleX :size="14" class="shrink-0" />
      <span ref="label" class="whitespace-nowrap pl-1.5" :class="compact ? COMPACT_LABEL_CLASS : ''">{{ labelText }}</span>
      <Button size="sm" class="ml-1.5" @click="emit('retry')">Retry</Button>
    </span>
    <Dropdown
      v-else-if="state.kind === 'ready' || state.kind === 'unavailable'"
      :overlay-store="appOverlayStore"
      variant="ghost"
      trigger-label="Model"
      placement="bottom-end"
    >
      <template #trigger="{ isOpen }">
        <span class="relative inline-flex items-center overflow-hidden">
          <!-- The sparkle stands in for the name; with room for the name it would only repeat it. -->
          <Sparkles
            v-if="compact"
            :size="ICON_PX.in28"
            class="mr-1 shrink-0"
          />
          <!-- The name sits a step above the chip's tone and the level a step below it, so the
               two stay two steps apart whether the chip is subtle (closed) or body (open). -->
          <span
            ref="label"
            class="inline-flex items-center gap-1 whitespace-nowrap"
            :class="compact ? COMPACT_LABEL_CLASS : ''"
          >
            <span :class="isOpen ? 'text-fg-body' : 'text-fg-muted'">{{ state.label }}</span>
            <span
              v-if="reasoningLabel"
              :class="isOpen ? 'text-fg-subtle' : 'text-fg-faint'"
            >{{ reasoningLabel }}</span>
          </span>
          <Zap v-if="fast" :size="ICON_PX.in28" class="ml-1 shrink-0" />
          <Tooltip v-if="unavailable" content="This model is unavailable. Choose another to send.">
            <TriangleAlert
              :size="ICON_PX.in28"
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
        />
      </template>
    </Dropdown>
  </Tooltip>
</template>
