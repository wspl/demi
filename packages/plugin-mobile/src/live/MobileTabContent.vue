<script setup lang="ts">
import { computed } from 'vue'
import { IconButton, Tooltip, downloadUrl, usePage } from '@demicodes/plugin-sdk'
import DeviceView from './DeviceView.vue'
import type { MobileDevicesController } from './controller'
import { DEVICE_CONTROLS, type DeviceControl } from './controls'
import type { MobileTabData } from './kind'
import { t } from '../locales/text'

/**
 * A device's tab (`mobile.md` § The live view): its toolbar, which starts
 * with the device's name where the whole name fits, never on a phone,
 * whose tab names the device, and holds the panel's controls, each the
 * operation of the command that does the same, and the device live. The
 * agent opens the device; the tab goes when the device closes.
 */
const props = defineProps<{
  conversation: string
  session: MobileDevicesController
  tabId: string
  data: MobileTabData
  shown: boolean
}>()

const { files, errors } = usePage()
/** Whether the device records, as the device says, whoever started it. */
const recording = computed(() => props.session.session.recording(props.data.device))

/** The file a control left on the Host goes to the user's computer, as a download does. */
function save(path: string | null): void {
  if (!path) {
    return
  }
  const contents = files(props.conversation).workspace?.source.contents
  if (!contents) {
    errors.report(t('failures.save'), new Error(t('failures.hostFilesUnreachable')))
    return
  }
  downloadUrl(contents.url(path, { download: true }))
}

/** A toolbar control: its call, and the file it left saved on this computer. */
async function act(control: DeviceControl): Promise<void> {
  save(await props.session.run(control.call(props.data.device, { recording: recording.value })))
}
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col">
    <div class="flex h-10 flex-none items-center gap-0.5 border-b border-line px-2">
      <!-- The name shows whole or not at all: beside the empty first item it
           fits on the one line, or wraps to the next, which the row cuts off.
           A phone's tab already names the device. -->
      <div class="flex h-10 min-w-0 flex-1 flex-wrap items-center overflow-hidden phone:invisible">
        <span class="h-10 w-0" />
        <span class="whitespace-nowrap px-2 text-[13px] leading-10 text-fg-body">{{ data.name }}</span>
      </div>
      <template v-for="(control, index) in DEVICE_CONTROLS" :key="index">
        <div v-if="control.group" class="mx-1 h-4 w-px flex-none bg-line" />
        <Tooltip :content="t(control.label({ recording }))">
          <IconButton
            class="flex-none"
            :icon="control.icon({ recording })"
            variant="ghost"
            size="sm"
            :pressed="control.pressed?.({ recording })"
            :indicator="control.pressed?.({ recording }) ? 'danger' : undefined"
            :aria-label="t(control.label({ recording }))"
            @click="act(control)"
          />
        </Tooltip>
      </template>
    </div>
    <div
      v-if="session.pictures.value === 'unsupported'"
      class="flex min-h-0 flex-1 items-center justify-center px-6 text-center text-[13px] text-fg-faint"
    >
      {{ t('region.unsupported') }}
    </div>
    <DeviceView v-else :session="session.session" :device="data.device" :shown="shown" />
  </div>
</template>
