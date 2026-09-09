<script setup lang="ts">
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import Dropdown from '../ui/Dropdown.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import Switch from '../ui/Switch.vue'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'

/** What is kept, for how long, and who can see it. */
defineProps<{
  overlayStore: OverlayStore
  /** Retention choices, formatted by the host (`Forever`, `90 days`). */
  retentions: string[]
}>()

const retention = defineModel<string>('retention', { required: true })
const shareLinks = defineModel<boolean>('shareLinks', { required: true })
const telemetry = defineModel<boolean>('telemetry', { required: true })

const emit = defineEmits<{
  export: []
  deleteAll: []
}>()
</script>

<template>
  <SettingsPage
    title="Data & privacy"
    description="What is kept, for how long, and who can see it."
  >
    <SettingsGroup title="Conversations">
      <SettingsRow
        label="Keep transcripts for"
        description="Older conversations are deleted from your account."
      >
        <Dropdown
          size="sm"
          :overlay-store="overlayStore"
          variant="default"
          trigger-label="Retention"
        >
          <template #trigger>{{ retention }}</template>
          <template #content="{ close }">
            <Menu>
              <MenuItem
                v-for="entry in retentions"
                :key="entry"
                :label="entry"
                choice
                :is-selected="retention === entry"
                @select="retention = entry; close()"
              />
            </Menu>
          </template>
        </Dropdown>
      </SettingsRow>
      <SettingsRow
        label="Share links"
        description="Let a conversation be published at a public URL."
      >
        <Switch v-model="shareLinks" />
      </SettingsRow>
      <SettingsRow
        label="Export everything"
        description="Transcripts and settings as a zip. Ready in a few minutes."
      >
        <Button size="sm" @click="emit('export')">Request export</Button>
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="Diagnostics">
      <SettingsRow
        label="Send usage data"
        description="Crashes and feature usage. Never prompts, transcripts or file contents."
      >
        <Switch v-model="telemetry" />
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="Danger zone">
      <SettingsRow
        label="Delete all conversations"
        description="From your account. Projects and settings stay."
      >
        <Button
          size="sm"
          variant="danger"
          @click="emit('deleteAll')"
        >Delete all</Button>
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>
</template>
