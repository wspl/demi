<script setup lang="ts">
import { WifiOff } from '@lucide/vue'
import { t } from './locales/text'

/**
 * What the work panel shows in place of its tabs' contents while the
 * conversation's primary Host is a paired device whose runner is not connected:
 * which Host it is, and that its files, changes and browser come back when it
 * does. It never spins, since nothing it waits for is under way; a tab's own
 * last view is kept behind it and shows again at once.
 */
defineProps<{
  /** The offline Host's name. */
  name: string
}>()
</script>

<template>
  <div
    class="flex flex-1 select-none flex-col items-center justify-center gap-1 px-6 text-center"
    role="status"
  >
    <WifiOff :size="20" class="mb-1 text-fg-faint" aria-hidden="true" />
    <span class="text-[13px] text-fg-muted">{{ t('panel.hostOffline', { name }) }}</span>
    <span class="text-caption text-fg-faint">{{ t('panel.hostOfflineDetail') }}</span>
  </div>
</template>
