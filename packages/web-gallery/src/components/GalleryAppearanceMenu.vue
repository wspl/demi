<script setup lang="ts">
import { Palette } from '@lucide/vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import {
  applyParadigm,
  galleryState,
  PARADIGMS,
  type ParadigmId,
} from '../gallery-state'
import AccentPicker from './AccentPicker.vue'
import AxisPicker from './AxisPicker.vue'

const paradigmIds = PARADIGMS.map((item) => item.id)
const paradigmNames = Object.fromEntries(
  PARADIGMS.map((item) => [item.id, item.name]),
) as Record<ParadigmId, string>

function setMode(mode: 'light' | 'dark') {
  galleryState.mode = mode
}
</script>

<template>
  <Dropdown
    :overlay-store="appOverlayStore"
    placement="bottom-end"
  >
    <template #trigger="{ isOpen }">
      <IconButton
        :icon="Palette"
        variant="ghost"
        :pressed="isOpen"
        aria-label="Appearance"
      />
    </template>
    <template #content>
      <div
        class="overlay-shell overlay-panel flex max-h-[min(36rem,calc(100vh-2rem))] w-72 flex-col gap-3 overflow-y-auto p-3"
      >
        <AxisPicker
          label="Paradigm"
          :values="paradigmIds"
          :names="paradigmNames"
          :model-value="
            galleryState.paradigm === 'custom' ? undefined : galleryState.paradigm
          "
          @update:model-value="applyParadigm"
        />
        <p
          v-if="galleryState.paradigm === 'custom'"
          class="text-[12px] text-fg-muted"
        >
          Custom
        </p>
        <AxisPicker
          label="Mode"
          :values="['dark', 'light'] as const"
          :model-value="galleryState.mode"
          @update:model-value="setMode"
        />
        <AxisPicker
          label="Tone"
          :values="['zinc', 'cool', 'warm', 'ink'] as const"
          :model-value="galleryState.tone"
          @update:model-value="galleryState.tone = $event"
        />
        <AccentPicker
          :model-value="galleryState.accent"
          @update:model-value="galleryState.accent = $event"
        />
        <AxisPicker
          label="Density"
          :values="['compact', 'regular', 'comfortable'] as const"
          :model-value="galleryState.density"
          @update:model-value="galleryState.density = $event"
        />
        <AxisPicker
          label="Radius"
          :values="['tight', 'medium', 'soft'] as const"
          :model-value="galleryState.radius"
          @update:model-value="galleryState.radius = $event"
        />
        <AxisPicker
          label="Shadow"
          :values="['hairline', 'soft', 'carved'] as const"
          :model-value="galleryState.shadow"
          @update:model-value="galleryState.shadow = $event"
        />
      </div>
    </template>
  </Dropdown>
</template>
