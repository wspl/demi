<script setup lang="ts">
import { ref } from 'vue'
import { Monitor, Moon, RefreshCw, Sun } from '@lucide/vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsGroup from '@demicodes/web-ui/settings/SettingsGroup.vue'
import ProviderQuota from '@demicodes/web-ui/settings/ProviderQuota.vue'
import SettingsRow from '@demicodes/web-ui/settings/SettingsRow.vue'
import type { SettingsQuotaWindow } from '@demicodes/web-ui/settings/types'
import Button from '@demicodes/web-ui/ui/Button.vue'
import CommitTextInput from '@demicodes/web-ui/ui/CommitTextInput.vue'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import Segmented, { type SegmentedOption } from '@demicodes/web-ui/ui/Segmented.vue'
import Slider from '@demicodes/web-ui/ui/Slider.vue'
import Switch from '@demicodes/web-ui/ui/Switch.vue'
import Tag from '@demicodes/web-ui/ui/Tag.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import { productWould } from '../product-would'

/**
 * One row of each kind of control a settings card holds, as the product's
 * pages place them. Each control acts on the specimen's own state.
 */
type Theme = 'light' | 'dark' | 'system'
const themes: readonly SegmentedOption<Theme>[] = [
  { value: 'light', label: 'Light', icon: Sun },
  { value: 'dark', label: 'Dark', icon: Moon },
  { value: 'system', label: 'System', icon: Monitor },
]
const protocols = ['Chat Completions', 'Responses', 'Messages'] as const

const name = ref('explore')
const baseUrl = ref('https://api.example.com/v1')
const protocol = ref<(typeof protocols)[number]>('Responses')
const fontSize = ref(14)
const theme = ref<Theme>('system')
const sound = ref(true)
/** The account row's usage, which Refresh reads again: each window a little further used. */
const usage = ref<SettingsQuotaWindow[]>([
  { id: 'five_hour', label: '5h session', used: 62, max: 100, resets: 'in 2 h 10 min' },
  { id: 'seven_day', label: '7d all models', used: 31, max: 100, resets: 'Monday' },
])
function refreshUsage(): void {
  usage.value = usage.value.map((window) => ({ ...window, used: Math.min(window.max, window.used + 3) }))
}
</script>

<template>
  <SettingsGroup title="Every Control">
    <SettingsRow label="Name" description="Lowercase letters, digits and hyphens.">
      <TextInput v-model="name" aria-label="Name" class="w-48 max-w-full" />
    </SettingsRow>
    <SettingsRow label="Base URL">
      <CommitTextInput
        :model-value="baseUrl"
        aria-label="Base URL"
        class="w-72 max-w-full"
        @commit="baseUrl = $event"
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
    <SettingsRow label="Protocol">
      <Dropdown
        size="sm"
        :overlay-store="appOverlayStore"
        variant="default"
        trigger-label="Protocol"
      >
        <template #trigger>{{ protocol }}</template>
        <template #content="{ close }">
          <Menu>
            <MenuItem
              v-for="entry in protocols"
              :key="entry"
              :label="entry"
              choice
              :is-selected="protocol === entry"
              @select="protocol = entry; close()"
            />
          </Menu>
        </template>
      </Dropdown>
    </SettingsRow>
    <SettingsRow label="Theme">
      <Segmented v-model="theme" size="sm" :options="themes" />
    </SettingsRow>
    <SettingsRow label="Sound" description="When a turn finishes in the background.">
      <Switch v-model="sound" aria-label="Sound" />
    </SettingsRow>
    <SettingsRow label="Export" description="A copy of every conversation, sent by email.">
      <Button size="sm" @click="productWould('Request an Export')">Request Export</Button>
    </SettingsRow>
    <SettingsRow label="zan@example.com">
      <template #tags><Tag>Max 5×</Tag></template>
      <Tooltip content="Refresh usage">
        <IconButton size="sm" :icon="RefreshCw" aria-label="Refresh usage" spin-on-click @click="refreshUsage" />
      </Tooltip>
      <template #below>
        <ProviderQuota :windows="usage" :auto-refresh="false" @refresh="refreshUsage" />
      </template>
    </SettingsRow>
  </SettingsGroup>
</template>
