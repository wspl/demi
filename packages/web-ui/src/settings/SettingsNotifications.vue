<script setup lang="ts">
import { computed, ref } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import Switch from '../ui/Switch.vue'
import HomeScreenSheet from './HomeScreenSheet.vue'
import { t } from './locales/text'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'

/**
 * Settings › Notifications (`turns.md` § Notifications): whether this
 * browser notifies at all, by push, so with every Demi page closed too, and
 * what notifies, the agent's own notifications among it. The setting
 * belongs to the browser; turning it on asks the browser for permission,
 * and a refusal turns the switch back off and says how to allow
 * notifications again. Safari on an iPhone or iPad pushes only to Demi
 * added to the Home Screen: there turning it on shows how to add it
 * instead, and the row then says so with How to Add.
 */
const props = defineProps<{
  /**
   * What the browser allows this site: `default` until it was asked,
   * `unsupported` for a browser that cannot notify.
   */
  permission: NotificationPermission | 'unsupported'
  /** Asks the browser for permission to notify, answering what it decided. */
  requestPermission: () => Promise<NotificationPermission>
  /**
   * Safari on an iPhone or iPad, outside the Home Screen, which cannot push:
   * Demi's Home Screen icon, which the steps to add Demi show; none
   * elsewhere.
   */
  homeScreenIcon?: string
  overlayStore: OverlayStore
}>()
/** The user turned notifications on in this browser. */
const enabled = defineModel<boolean>('enabled', { required: true })
const turnFinishes = defineModel<boolean>('turnFinishes', { required: true })
const turnFails = defineModel<boolean>('turnFails', { required: true })
const needsPermission = defineModel<boolean>('needsPermission', { required: true })
/** The agent's own notifications, `demi notify send`. */
const agentNotifies = defineModel<boolean>('agentNotifies', { required: true })

/** The steps to add Demi to the Home Screen are open. */
const sheetOpen = ref(false)
/** The steps were shown once: the row says what to do instead of offering the switch. */
const toldHowToAdd = ref(false)

/** The browser cannot push; Safari outside the Home Screen can once Demi is added there. */
const unsupported = computed(() => props.permission === 'unsupported' && !props.homeScreenIcon)
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
  if (value && props.homeScreenIcon) {
    sheetOpen.value = true
    toldHowToAdd.value = true
    return
  }
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
        v-if="homeScreenIcon && toldHowToAdd"
        :label="t('notifications.homeScreen.row')"
      >
        <Button size="sm" @click="sheetOpen = true">{{ t('notifications.homeScreen.howTo') }}</Button>
      </SettingsRow>
      <SettingsRow
        v-else
        :label="t('notifications.browser.label')"
        :description="description"
      >
        <Switch
          :model-value="on"
          :disabled="unsupported || asking"
          :disabled-reason="unsupported ? t('notifications.browser.unsupported') : undefined"
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
      <SettingsRow :label="t('notifications.when.agentNotifies')">
        <Switch
          v-model="agentNotifies"
          :disabled="!on"
          :disabled-reason="t('notifications.when.turnOnFirst')"
        />
      </SettingsRow>
    </SettingsGroup>
    <HomeScreenSheet
      v-if="homeScreenIcon"
      :open="sheetOpen"
      :overlay-store="overlayStore"
      :app-icon="homeScreenIcon"
      @close="sheetOpen = false"
    />
  </SettingsPage>
</template>
