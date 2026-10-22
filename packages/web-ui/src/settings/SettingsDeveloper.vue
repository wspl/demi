<script setup lang="ts">
import Switch from '../ui/Switch.vue'
import { t } from './locales/text'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'

/**
 * Settings › Developer: what only whoever runs the server needs. For now, whether
 * the page shows the errors it logs, each with the ID that finds its line in the log.
 */

/** Whether a defect the page found itself is shown with the ID of its line in the log. */
const showUnexpectedErrors = defineModel<boolean>('showUnexpectedErrors', { required: true })
</script>

<template>
  <SettingsPage :title="t('developer.title')">
    <SettingsGroup>
      <SettingsRow
        :label="t('developer.showUnexpectedErrors')"
        :description="t('developer.showUnexpectedErrorsDescription')"
      >
        <Switch
          v-model="showUnexpectedErrors"
          size="sm"
          :aria-label="t('developer.showUnexpectedErrors')"
        />
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>
</template>
