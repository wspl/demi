<script setup lang="ts">
import { computed } from 'vue'
import { Monitor, Moon, Sun } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import type { ThemeChoice } from '../theme/appTheme'
import { PRODUCT_TONES, productAccentOptions, toneName, type ProductAccent, type ProductTone } from '../theme/productAppearance'
import { LANGUAGES, LANGUAGE_NAMES, type LanguageChoice } from '../i18n/languages'
import { t } from '../locales/text'
import Dropdown from '../ui/Dropdown.vue'
import Menu from '../ui/Menu.vue'
import MenuDivider from '../ui/MenuDivider.vue'
import MenuItem from '../ui/MenuItem.vue'
import Segmented, { type SegmentedOption } from '../ui/Segmented.vue'
import Slider from '../ui/Slider.vue'
import SwatchPicker from '../ui/SwatchPicker.vue'
import AppearancePreview from './AppearancePreview.vue'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'

/**
 * Language and look. Every row is a model; the preview beside the look
 * reads the live tokens. The language menu names each language in its own language,
 * and System in the page's, as macOS's Language & Region does.
 */
defineProps<{
  overlayStore: OverlayStore
}>()

const language = defineModel<LanguageChoice>('language', { required: true })
const theme = defineModel<ThemeChoice>('theme', { required: true })
const tone = defineModel<ProductTone>('tone', { required: true })
const accent = defineModel<ProductAccent>('accent', { required: true })
const fontSize = defineModel<number>('fontSize', { required: true })

/** A choice's name in the menu and on its button. */
function choiceName(choice: LanguageChoice): string {
  return choice === 'system' ? t('settings.general.system') : LANGUAGE_NAMES[choice]
}

const themeOptions = computed<readonly SegmentedOption<ThemeChoice>[]>(() => [
  { value: 'light', label: t('settings.general.light'), icon: Sun },
  { value: 'dark', label: t('settings.general.dark'), icon: Moon },
  { value: 'system', label: t('settings.general.system'), icon: Monitor },
])
const toneOptions = computed(() => PRODUCT_TONES.map((id) => ({ value: id, label: toneName(id) })))
</script>

<template>
  <SettingsPage :title="t('settings.general.title')" :description="t('settings.general.description')">
    <SettingsGroup :title="t('settings.general.appearance')">
      <template #aside><AppearancePreview :font-size="fontSize" /></template>
      <SettingsRow :label="t('settings.general.language')">
        <Dropdown
          size="sm"
          :overlay-store="overlayStore"
          variant="default"
          :trigger-label="t('settings.general.language')"
        >
          <template #trigger>{{ choiceName(language) }}</template>
          <template #content="{ close }">
            <Menu>
              <MenuItem
                :label="choiceName('system')"
                choice
                :is-selected="language === 'system'"
                @select="language = 'system'; close()"
              />
              <MenuDivider />
              <MenuItem
                v-for="entry in LANGUAGES"
                :key="entry"
                :label="choiceName(entry)"
                :lang="entry"
                choice
                :is-selected="language === entry"
                @select="language = entry; close()"
              />
            </Menu>
          </template>
        </Dropdown>
      </SettingsRow>
      <SettingsRow :label="t('settings.general.theme')">
        <Segmented
          v-model="theme"
          size="sm"
          :options="themeOptions"
        />
      </SettingsRow>
      <SettingsRow :label="t('settings.general.tone')">
        <Segmented
          size="sm"
          :model-value="tone"
          :options="toneOptions"
          @update:model-value="tone = $event as ProductTone"
        />
      </SettingsRow>
      <SettingsRow :label="t('settings.general.accent')">
        <SwatchPicker
          :model-value="accent"
          :options="productAccentOptions()"
          @update:model-value="accent = $event as ProductAccent"
        />
      </SettingsRow>
      <SettingsRow :label="t('settings.general.textSize')">
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
