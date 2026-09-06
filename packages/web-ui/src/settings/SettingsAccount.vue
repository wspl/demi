<script setup lang="ts">
import type { OverlayStore } from '../overlay/overlayStore'
import type { ThemeMode } from '../theme/appTheme'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'

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
  <h3>Your workspace</h3>
  <label>
    Display name
    <TextInput v-model="name" maxlength="50" />
  </label>
  <div class="setting-row">
    <strong>Appearance</strong>
    <Dropdown :overlay-store="overlayStore" variant="default" trigger-label="Appearance">
      <template #trigger>{{ theme === 'light' ? 'Light' : 'Dark' }}</template>
      <template #content>
        <Menu>
          <MenuItem label="Light" choice :is-selected="theme === 'light'" @select="emit('changeTheme', 'light')" />
          <MenuItem label="Dark" choice :is-selected="theme === 'dark'" @select="emit('changeTheme', 'dark')" />
        </Menu>
      </template>
    </Dropdown>
  </div>
  <div class="setting-row">
    <strong>Session</strong>
    <Button @click="emit('signOut')">Sign out</Button>
  </div>
</template>
