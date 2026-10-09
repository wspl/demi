<script setup lang="ts">
import { Monitor, Moon, Sun } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import type { ThemeChoice } from '../theme/appTheme'
import { PRODUCT_ACCENTS, PRODUCT_TONES, type ProductAccent, type ProductTone } from '../theme/productAppearance'
import Dropdown from '../ui/Dropdown.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import Segmented, { type SegmentedOption } from '../ui/Segmented.vue'
import Slider from '../ui/Slider.vue'
import SwatchPicker from '../ui/SwatchPicker.vue'
import AppearancePreview from './AppearancePreview.vue'
import { SEND_WAY_LABELS, otherWayKeys, type SendWhileRunning } from '../agent/send-way'
import { IN_DEVELOPMENT } from '../ui/disabled'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'

/**
 * Language, look, and what Enter does with a message while the agent works.
 * Every row is a model; the preview beside the look reads the live tokens.
 */
defineProps<{
  overlayStore: OverlayStore
  languages: string[]
}>()

const language = defineModel<string>('language', { required: true })
const theme = defineModel<ThemeChoice>('theme', { required: true })
const tone = defineModel<ProductTone>('tone', { required: true })
const accent = defineModel<ProductAccent>('accent', { required: true })
const fontSize = defineModel<number>('fontSize', { required: true })
const sendWhileRunning = defineModel<SendWhileRunning>('sendWhileRunning', { required: true })

const themeOptions: readonly SegmentedOption<ThemeChoice>[] = [
  { value: 'light', label: 'Light', icon: Sun },
  { value: 'dark', label: 'Dark', icon: Moon },
  { value: 'system', label: 'System', icon: Monitor },
]
const toneOptions = PRODUCT_TONES.map((entry) => ({ value: entry.id, label: entry.label }))
const sendOptions: readonly SegmentedOption<SendWhileRunning>[] = [
  { value: 'steer', label: SEND_WAY_LABELS.steer },
  { value: 'queue', label: SEND_WAY_LABELS.queue },
]
const sendDescription = `Steer: the agent reads your message in the running turn. Queue: it waits for the turn to end. ${otherWayKeys()} sends one message the other way.`
</script>

<template>
  <SettingsPage title="General" description="Language, look and messages.">
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
          v-model="theme"
          size="sm"
          :options="themeOptions"
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
    <SettingsGroup title="Messages">
      <SettingsRow label="Enter while the agent works" :description="sendDescription">
        <Segmented
          v-model="sendWhileRunning"
          size="sm"
          :options="sendOptions"
        />
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>
</template>
