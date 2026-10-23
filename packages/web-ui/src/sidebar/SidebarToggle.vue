<script setup lang="ts">
import { computed, inject } from 'vue'
import { PanelLeft } from '@lucide/vue'
import IconButton from '../ui/IconButton.vue'
import Tooltip from '../ui/Tooltip.vue'
import { usePhone } from '../ui/phone'
import { sidebarDrawerKey } from './sidebar-drawer'
import { t } from './locales/text'

/**
 * The button that collapses a computer's sidebar and brings it back, drawn as the
 * Panel Left icon the phone's drawer button uses. One component and one rule for where
 * it shows, whichever surface holds it:
 *
 * - `sidebar`: the sidebar's top row, always (Hide Sidebar).
 * - `bar`: a page's top bar, left of its title, while the sidebar is collapsed (Show
 *   Sidebar).
 *
 * It shows nowhere on a phone, whose sidebar is the drawer, nor outside an app frame.
 */
const props = defineProps<{ placement: 'sidebar' | 'bar' }>()
const frame = inject(sidebarDrawerKey, null)
const phone = usePhone()

const shown = computed(() =>
  frame !== null && !phone.value && (props.placement === 'sidebar' || frame.collapsed()),
)
const label = computed(() => (frame?.collapsed() ? t('layout.showSidebar') : t('layout.hideSidebar')))
</script>

<template>
  <Tooltip v-if="shown && frame" class="shrink-0" placement="bottom">
    <IconButton :icon="PanelLeft" variant="ghost" :aria-label="label" @click="frame.toggle()" />
    <template #overlay>
      <span class="flex items-center gap-2">
        {{ label }}
        <span v-if="frame.shortcut()" class="text-fg-subtle">{{ frame.shortcut() }}</span>
      </span>
    </template>
  </Tooltip>
</template>
