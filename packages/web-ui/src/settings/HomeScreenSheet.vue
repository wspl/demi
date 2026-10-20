<script setup lang="ts">
import { Share, SquarePlus } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import Dialog from '../ui/Dialog.vue'
import Sheet from '../ui/Sheet.vue'
import { usePhone } from '../ui/phone'
import { t } from './locales/text'

/**
 * How to get notifications on an iPhone or iPad. Safari sends push notifications
 * only to Demi added to the Home Screen, so turning them on in Safari shows these
 * three steps, each beside the icon it names, instead of the browser's question.
 *
 * It is a sheet on a phone, a dialog on an iPad's wider screen.
 */
defineProps<{
  open: boolean
  overlayStore: OverlayStore
  /** Demi's Home Screen icon, which the last step names. */
  appIcon: string
}>()

const emit = defineEmits<{
  close: []
}>()

const phone = usePhone()
</script>

<template>
  <component
    :is="phone ? Sheet : Dialog"
    v-bind="phone ? { open, label: t('notifications.homeScreen.title') } : { isOpen: open, overlayStore, label: t('notifications.homeScreen.title') }"
    @close="emit('close')"
  >
    <div class="flex flex-col gap-5 px-5 pb-5 pt-5 phone:pb-[calc(1.25rem+var(--safe-bottom))] phone:pt-2">
      <h3 class="pr-8 text-[15px] font-medium leading-5 text-fg-emphasis phone:pr-0 phone:text-center phone:text-[17px] phone:leading-6">
        {{ t('notifications.homeScreen.title') }}
      </h3>
      <ol class="flex flex-col gap-4">
        <li class="flex items-center gap-3">
          <span class="flex size-10 shrink-0 items-center justify-center rounded-xl bg-surface-card text-accent">
            <Share :size="20" aria-hidden="true" />
          </span>
          <span class="text-explanation leading-5 text-fg-body">{{ t('notifications.homeScreen.share') }}</span>
        </li>
        <li class="flex items-center gap-3">
          <span class="flex size-10 shrink-0 items-center justify-center rounded-xl bg-surface-card text-fg">
            <SquarePlus :size="20" aria-hidden="true" />
          </span>
          <span class="text-explanation leading-5 text-fg-body">{{ t('notifications.homeScreen.add') }}</span>
        </li>
        <li class="flex items-center gap-3">
          <!-- On the same card as the other steps' glyphs, and about as large as they look. -->
          <span class="flex size-10 shrink-0 items-center justify-center rounded-xl bg-surface-card">
            <img :src="appIcon" alt="" class="size-7 rounded-[0.45rem]">
          </span>
          <span class="text-explanation leading-5 text-fg-body">{{ t('notifications.homeScreen.open') }}</span>
        </li>
      </ol>
      <Button v-if="phone" variant="primary" size="lg" class="w-full" @click="emit('close')">{{ t('notifications.homeScreen.ok') }}</Button>
    </div>
    <template v-if="!phone" #footer>
      <Button variant="primary" @click="emit('close')">{{ t('notifications.homeScreen.ok') }}</Button>
    </template>
  </component>
</template>
