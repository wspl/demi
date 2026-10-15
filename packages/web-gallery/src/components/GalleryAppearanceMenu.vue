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
import { t as ui } from '@demicodes/web-ui/locales/text'
import { t } from '../locales/text'
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
        :aria-label="ui('settings.general.appearance')"
      />
    </template>
    <template #content>
      <div
        class="overlay-shell flex max-h-[min(36rem,calc(100vh-2rem))] w-72 flex-col overflow-hidden"
      >
        <ScrollArea viewport-class="flex flex-col gap-3 p-3">
          <AxisPicker
            :label="ui('settings.general.theme')"
            :values="paradigmIds"
            :names="paradigmNames"
            :model-value="galleryState.paradigm"
            @update:model-value="applyParadigm"
          />
          <AxisPicker
            :label="t('chrome.mode')"
            :values="['dark', 'light'] as const"
            :names="{ dark: ui('settings.general.dark'), light: ui('settings.general.light') }"
            :model-value="galleryState.mode"
            @update:model-value="setMode"
          />
          <AccentPicker
            :model-value="galleryState.accent"
            @update:model-value="galleryState.accent = $event"
          />
          <!-- On trial: the product's face is still being chosen. -->
          <AxisPicker
            :label="t('chrome.font')"
            :values="FONT_IDS"
            :names="{ system: ui('settings.general.system'), inter: 'Inter', geist: 'Geist' }"
            :model-value="galleryState.font"
            @update:model-value="galleryState.font = $event"
          />
        </ScrollArea>
      </div>
    </template>
  </Dropdown>
</template>
