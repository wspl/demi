<script setup lang="ts">
import { computed, ref } from 'vue'
import Switch from '../ui/Switch.vue'
import { t } from './locales/text'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'

/**
 * Settings › Notifications (`turns.md` § Notifications): whether this
 * browser notifies at all, and what notifies. The setting belongs to the
 * browser, which the host keeps it in; turning it on asks the browser for
 * permission, and a refusal turns the switch back off and says how to allow
 * notifications again.
 */
const props = defineProps<{
  /**
   * What the browser allows this site: `default` until it was asked,
   * `unsupported` for a browser that cannot notify.
   */
  permission: NotificationPermission | 'unsupported'
  /** Asks the browser for permission to notify, answering what it decided. */
  requestPermission: () => Promise<NotificationPermission>
}>()
/** The user turned notifications on in this browser. */
const enabled = defineModel<boolean>('enabled', { required: true })
const turnFinishes = defineModel<boolean>('turnFinishes', { required: true })
const turnFails = defineModel<boolean>('turnFails', { required: true })
const needsPermission = defineModel<boolean>('needsPermission', { required: true })

/** The browser is asking the user: the switch shows on until it answers. */
const asking = ref(false)
/** Notifications come: the user turned them on, and the browser allows them. */
const on = computed(() => asking.value || (enabled.value && props.permission === 'granted'))
const description = computed(() => {
  if (props.permission === 'denied') {
    return t('notifications.browser.blocked')
  }
  return t('notifications.browser.thisBrowser')
})

async function turn(value: boolean): Promise<void> {
  if (!value) {
    enabled.value = false
    return
  }
  if (props.permission === 'granted') {
    enabled.value = true
    return
  }
  asking.value = true
  try {
    enabled.value = (await props.requestPermission()) === 'granted'
  } finally {
    asking.value = false
  }
}
</script>

<template>
  <SettingsPage
    :title="t('notifications.title')"
    :description="t('notifications.description')"
  >
    <SettingsGroup :title="t('notifications.browser.title')">
      <SettingsRow
        :label="t('notifications.browser.label')"
        :description="description"
      >
        <Switch
          :model-value="on"
          :disabled="permission === 'unsupported' || asking"
          :disabled-reason="permission === 'unsupported' ? t('notifications.browser.unsupported') : undefined"
          @update:model-value="turn"
        />
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup :title="t('notifications.when.title')">
      <SettingsRow :label="t('notifications.when.turnFinishes')">
        <Switch
          v-model="turnFinishes"
          :disabled="!on"
          :disabled-reason="t('notifications.when.turnOnFirst')"
        />
      </SettingsRow>
      <SettingsRow :label="t('notifications.when.turnFails')">
        <Switch
          v-model="turnFails"
          :disabled="!on"
          :disabled-reason="t('notifications.when.turnOnFirst')"
        />
      </SettingsRow>
      <SettingsRow :label="t('notifications.when.needsPermission')">
        <Switch
          v-model="needsPermission"
          :disabled="!on"
          :disabled-reason="t('notifications.when.turnOnFirst')"
        />
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>
</template>
