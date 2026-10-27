<script setup lang="ts">
import { RegionStatus, type HostDownload } from '@demicodes/plugin-sdk'
import { deviceStatus, type ShownPictures } from './status'
import { t } from '../locales/text'

/**
 * What a device's view says over it (`./status.ts`): a picture or a frame
 * that cannot show, with Retry for the frame; the wait before the first
 * picture, unless the view says it on the device's own screen; and a
 * stream that stalls or opens again under the last picture.
 */
const props = defineProps<{
  session: ShownPictures
  downloads: readonly HostDownload[]
  /** Whether the view says the wait on the device's screen itself. */
  waitOnScreen: boolean
}>()

const { progress, progressText, waitingLabel, stalled, reconnecting } = deviceStatus(props)
</script>

<template>
  <RegionStatus
    v-if="session.state.pictureless"
    class="absolute inset-0 bg-surface"
    status="failed"
    :label="t('region.couldntShow')"
    :detail="t(`reasons.${session.state.pictureless}`)"
  />
  <RegionStatus
    v-else-if="session.state.frameFailed"
    class="absolute inset-0 bg-surface"
    status="failed"
    :label="t('region.couldntShow')"
    :detail="t('region.frameFailed')"
    :on-retry="() => session.retryFrame()"
  />
  <RegionStatus
    v-else-if="waitingLabel && !waitOnScreen"
    class="absolute inset-0"
    status="loading"
    :label="waitingLabel"
    :progress="progress"
  />
  <div v-if="stalled || reconnecting" role="status" class="pointer-events-none absolute inset-x-0 bottom-0 flex justify-center p-2">
    <span class="rounded-md bg-surface px-2 py-1 text-[12px] text-fg-subtle shadow">{{ reconnecting ? (progress ? `${t('region.downloading')} ${progressText}` : t('region.reconnecting')) : t('region.waiting') }}</span>
  </div>
</template>
