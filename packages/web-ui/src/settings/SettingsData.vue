<script setup lang="ts">
import Button from '../ui/Button.vue'
import Switch from '../ui/Switch.vue'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'

// What is kept and who can see it.
const telemetry = defineModel<boolean>('telemetry', { required: true })

const emit = defineEmits<{
  deleteAll: []
}>()
</script>

<template>
  <SettingsPage
    title="Data & Privacy"
    description="What is kept and who can see it."
  >
    <SettingsGroup title="Diagnostics">
      <SettingsRow
        label="Send usage data"
        description="Crashes and feature usage. Never prompts, transcripts or file contents."
      >
        <Switch v-model="telemetry" />
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="Danger Zone">
      <SettingsRow
        label="Delete all conversations"
        description="From your account. Projects and settings stay."
      >
        <Button
          size="sm"
          variant="danger"
          @click="emit('deleteAll')"
        >Delete All</Button>
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>
</template>
