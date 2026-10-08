<script setup lang="ts">
import { Palette } from '@lucide/vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import ScrollArea from '@demicodes/web-ui/ui/ScrollArea.vue'
import {
  applyParadigm,
  FONT_IDS,
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
        class="overlay-shell overlay-panel flex max-h-[min(36rem,calc(100vh-2rem))] w-72 flex-col overflow-hidden"
      >
        <ScrollArea viewport-class="flex flex-col gap-3 p-3">
          <AxisPicker
            label="Theme"
            :values="paradigmIds"
            :names="paradigmNames"
            :model-value="galleryState.paradigm"
            @update:model-value="applyParadigm"
          />
          <AxisPicker
            label="Mode"
            :values="['dark', 'light'] as const"
            :model-value="galleryState.mode"
            @update:model-value="setMode"
          />
          <AccentPicker
            :model-value="galleryState.accent"
            @update:model-value="galleryState.accent = $event"
          />
          <!-- On trial: the product's face is still being chosen. -->
          <AxisPicker
            label="Font"
            :values="FONT_IDS"
            :names="{ system: 'System', inter: 'Inter', geist: 'Geist' }"
            :model-value="galleryState.font"
            @update:model-value="galleryState.font = $event"
          />
        </ScrollArea>
      </div>
    </template>
  </Dropdown>
</template>
