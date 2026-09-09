<script setup lang="ts">
import Switch from '../ui/Switch.vue'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'

/** When the agent needs you, or is done. Four switches, all models. */
const browser = defineModel<boolean>('browser', { required: true })
const sound = defineModel<boolean>('sound', { required: true })
const onFinish = defineModel<boolean>('onFinish', { required: true })
const onError = defineModel<boolean>('onError', { required: true })
</script>

<template>
  <SettingsPage
    title="Notifications"
    description="When the agent needs you, or is done."
  >
    <SettingsGroup title="Delivery">
      <SettingsRow
        label="Browser notifications"
        description="The browser asks once. Delivered while this tab is hidden."
      >
        <Switch v-model="browser" />
      </SettingsRow>
      <SettingsRow
        label="Sound"
        description="Only while this tab is in the background."
      >
        <Switch v-model="sound" />
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup
      title="Notify me when"
      description="Only while the conversation is not in front."
    >
      <SettingsRow label="A turn finishes"><Switch v-model="onFinish" /></SettingsRow>
      <SettingsRow label="A turn fails"><Switch v-model="onError" /></SettingsRow>
    </SettingsGroup>
  </SettingsPage>
</template>
