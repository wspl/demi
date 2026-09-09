<script setup lang="ts">
import { Monitor, Moon, Sun } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import type { ThemeChoice } from '../theme/appTheme'
import { PRODUCT_ACCENTS, PRODUCT_TONES, type ProductAccent, type ProductTone } from '../theme/productAppearance'
import Dropdown from '../ui/Dropdown.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import Segmented from '../ui/Segmented.vue'
import Slider from '../ui/Slider.vue'
import SwatchPicker from '../ui/SwatchPicker.vue'
import AppearancePreview from './AppearancePreview.vue'
import { IN_DEVELOPMENT } from '../ui/disabled'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'

/** Language and look. Every row is a model; the preview beside them reads the live tokens. */
defineProps<{
  overlayStore: OverlayStore
  languages: string[]
}>()

const language = defineModel<string>('language', { required: true })
const theme = defineModel<ThemeChoice>('theme', { required: true })
const tone = defineModel<ProductTone>('tone', { required: true })
const accent = defineModel<ProductAccent>('accent', { required: true })
const fontSize = defineModel<number>('fontSize', { required: true })

const themeOptions = [
  { value: 'light', label: 'Light', icon: Sun },
  { value: 'dark', label: 'Dark', icon: Moon },
  { value: 'system', label: 'System', icon: Monitor },
] as const
const toneOptions = PRODUCT_TONES.map((entry) => ({ value: entry.id, label: entry.label }))
</script>

<template>
  <SettingsPage title="General" description="Language and look.">
    <SettingsGroup title="Appearance">
      <template #aside><AppearancePreview :font-size="fontSize" /></template>
      <SettingsRow
        label="Language"
        disabled
        :disabled-reason="IN_DEVELOPMENT"
      >
        <Dropdown
          size="sm"
          disabled
          :overlay-store="overlayStore"
          variant="default"
          trigger-label="Language"
        >
          <template #trigger>{{ language }}</template>
          <template #content="{ close }">
            <Menu>
              <MenuItem
                v-for="entry in languages"
                :key="entry"
                :label="entry"
                choice
                :is-selected="language === entry"
                @select="language = entry; close()"
              />
            </Menu>
          </template>
        </Dropdown>
      </SettingsRow>
      <SettingsRow label="Theme">
        <Segmented
          size="sm"
          :model-value="theme"
          :options="themeOptions"
          @update:model-value="theme = $event as ThemeChoice"
        />
      </SettingsRow>
      <SettingsRow label="Tone">
        <Segmented
          size="sm"
          :model-value="tone"
          :options="toneOptions"
          @update:model-value="tone = $event as ProductTone"
        />
      </SettingsRow>
      <SettingsRow label="Accent">
        <SwatchPicker
          :model-value="accent"
          :options="PRODUCT_ACCENTS"
          @update:model-value="accent = $event as ProductAccent"
        />
      </SettingsRow>
      <SettingsRow label="Transcript text size" description="Messages only.">
        <Slider
          v-model="fontSize"
          :min="12"
          :max="18"
          :value-label="`${fontSize}px`"
          class="w-48"
        />
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>
</template>
