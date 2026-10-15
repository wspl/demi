<script setup lang="ts">
import { Languages } from '@lucide/vue'
import { LANGUAGES, LANGUAGE_NAMES, type LanguageChoice } from '@demicodes/web-ui/i18n/languages'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuDivider from '@demicodes/web-ui/ui/MenuDivider.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import { t as ui } from '@demicodes/web-ui/locales/text'
import { galleryState } from '../gallery-state'

/**
 * The language the gallery speaks: System, the browser's, or one Demi
 * speaks, as the product's Settings › General chooses it; the specimen of
 * that page changes the same choice.
 */
function choose(choice: LanguageChoice, close: () => void): void {
  galleryState.language = choice
  close()
}
</script>

<template>
  <Dropdown :overlay-store="appOverlayStore">
    <template #trigger="{ isOpen }">
      <IconButton
        :icon="Languages"
        variant="ghost"
        :pressed="isOpen"
        :aria-label="ui('settings.general.language')"
      />
    </template>
    <template #content="{ close }">
      <Menu>
        <MenuItem
          :label="ui('settings.general.system')"
          choice
          :is-selected="galleryState.language === 'system'"
          @select="choose('system', close)"
        />
        <MenuDivider />
        <MenuItem
          v-for="language in LANGUAGES"
          :key="language"
          :label="LANGUAGE_NAMES[language]"
          :lang="language"
          choice
          :is-selected="galleryState.language === language"
          @select="choose(language, close)"
        />
      </Menu>
    </template>
  </Dropdown>
</template>
