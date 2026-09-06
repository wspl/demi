<script setup lang="ts">
import type { OverlayStore } from '../overlay/overlayStore'
import type { ThemeMode } from '../theme/appTheme'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'

defineProps<{
  overlayStore: OverlayStore
  theme: ThemeMode
}>()

const name = defineModel<string>('name', { default: '' })

const emit = defineEmits<{
  changeTheme: [mode: ThemeMode]
  signOut: []
}>()
</script>

<template>
  <SettingsPage title="Account" description="How you appear, and how the app looks to you.">
    <SettingsGroup title="Profile">
      <SettingsRow label="Display name" description="Shown on your messages and in the sidebar.">
        <TextInput v-model="name" maxlength="50" class="w-56" />
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="Appearance">
      <SettingsRow label="Theme" description="Light or dark. Follows this browser only.">
        <Dropdown :overlay-store="overlayStore" variant="default" trigger-label="Theme">
          <template #trigger>{{ theme === 'light' ? 'Light' : 'Dark' }}</template>
          <template #content>
            <Menu>
              <MenuItem label="Light" choice :is-selected="theme === 'light'" @select="emit('changeTheme', 'light')" />
              <MenuItem label="Dark" choice :is-selected="theme === 'dark'" @select="emit('changeTheme', 'dark')" />
            </Menu>
          </template>
        </Dropdown>
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="Session">
      <SettingsRow label="Sign out" description="Ends this browser's session. Conversations stay on the server.">
        <Button @click="emit('signOut')">Sign out</Button>
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>
</template>
