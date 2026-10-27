<script lang="ts">
/** What the card shows of the browser it is in. */
export interface NotificationsCardView {
  /** What this browser allows: `default` until it is asked. */
  permission: NotificationPermission | 'unsupported'
  /** Demi's icon, for the sheet that shows how to add it to the Home Screen; given only where that is what turns notifications on. */
  homeScreenIcon?: string
  /** Turning on is under way: the buttons wait for it. */
  turning?: boolean
}
</script>

<script setup lang="ts">
import { t } from './locales/text'
import { computed, ref } from 'vue'
import { BellRing } from '@lucide/vue'
import Button from '../ui/Button.vue'
import { appOverlayStore } from '../overlay/appOverlay'
import HomeScreenSheet from '../settings/HomeScreenSheet.vue'
import RequestFrame from './RequestFrame.vue'

/**
 * The card a conversation shows when its agent found that no browser of the
 * user takes its notifications, in the frame of a request, above the
 * composer.
 *
 * - Turn On Notifications turns them on in this browser, which asks for its
 *   permission first; in Safari outside the Home Screen it shows how to add
 *   Demi there instead.
 * - A browser that blocks them or cannot notify says so, without the button.
 * - Dismiss removes the card from this conversation for good.
 *
 * It holds no command: the agent was told already.
 */
const props = defineProps<NotificationsCardView>()

const emit = defineEmits<{
  turnOn: []
  dismiss: []
}>()

const sheetOpen = ref(false)

/** Why this browser cannot turn them on; null when it can. */
const cannot = computed(() => {
  if (props.homeScreenIcon) {
    return null
  }
  if (props.permission === 'denied') {
    return t('notificationsCard.blocked')
  }
  return props.permission === 'unsupported' ? t('notificationsCard.unsupported') : null
})

function turnOn(): void {
  if (props.homeScreenIcon) {
    sheetOpen.value = true
    return
  }
  emit('turnOn')
}
</script>

<template>
  <RequestFrame :title="t('notificationsCard.title')" :icon="BellRing" :count="1">
    <p class="pl-6 leading-5 text-fg-muted">
      {{ t('notificationsCard.body') }}
      <template v-if="cannot">
        <br>
        <span class="text-fg-emphasis">{{ cannot }}</span>
      </template>
    </p>
    <template #footer>
      <Button size="sm" :disabled="turning" @click="emit('dismiss')">{{ t('notificationsCard.dismiss') }}</Button>
      <Button
        v-if="!cannot"
        size="sm"
        variant="primary"
        :disabled="turning"
        @click="turnOn"
      >{{ t('notificationsCard.turnOn') }}</Button>
    </template>
    <HomeScreenSheet
      v-if="homeScreenIcon"
      :open="sheetOpen"
      :overlay-store="appOverlayStore"
      :app-icon="homeScreenIcon"
      @close="sheetOpen = false"
    />
  </RequestFrame>
</template>
