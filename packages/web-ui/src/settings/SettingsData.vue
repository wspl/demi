<script setup lang="ts">
import Button from '../ui/Button.vue'
import Switch from '../ui/Switch.vue'
import { t } from './locales/text'
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
    :title="t('data.title')"
    :description="t('data.description')"
  >
    <SettingsGroup :title="t('data.diagnostics.title')">
      <SettingsRow
        :label="t('data.diagnostics.send')"
        :description="t('data.diagnostics.sendDescription')"
      >
        <Switch v-model="telemetry" />
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup :title="t('data.danger.title')">
      <SettingsRow
        :label="t('data.danger.deleteAll')"
        :description="t('data.danger.deleteAllDescription')"
      >
        <Button
          size="sm"
          variant="danger"
          @click="emit('deleteAll')"
        >{{ t('data.danger.deleteAllButton') }}</Button>
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>
</template>
