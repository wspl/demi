<script setup lang="ts">
import { computed } from 'vue'
import { Toolbar, downloadUrl, usePage, type ToolbarAction } from '@demicodes/plugin-sdk'
import DeviceView from './DeviceView.vue'
import type { MobileDevicesController } from './controller'
import { deviceControls, type DeviceControl } from './controls'
import { MobilePlatformLogo, type MobileTabData } from './kind'
import { t } from '../locales/text'

/**
 * A device's tab (`packages/plugin-mobile/src/index.ts`): its toolbar, and the
 * device live. The agent opens the device; the tab goes when the device closes.
 *
 * The toolbar:
 *
 * - Starts with the device's system's logo and name where the whole name fits,
 *   then its model in grey where that fits too, never on a phone, whose tab
 *   names the device.
 * - Holds the panel's controls, each the operation of the command that does the
 *   same.
 */
const props = defineProps<{
  conversation: string
  session: MobileDevicesController
  tabId: string
  data: MobileTabData
  shown: boolean
}>()

const { files, errors, overlays } = usePage()
/** Whether the device records, as the device says, whoever started it. */
const recording = computed(() => props.session.session.recording(props.data.device))
/**
 * The device's model as its driver reads it: the live list's once it lists
 * the device, the tab's until then; none when the driver knows none.
 */
const model = computed(() => {
  const listed = props.session.session.state.devices.find((device) => device.id === props.data.device)
  return listed ? listed.model : props.data.model ?? null
})
/** The controls of the device's platform: Back on Android alone. */
const controls = computed(() => deviceControls(props.session.session.platform(props.data.device)))

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

/**
 * A toolbar control's call. Screenshot and Record leave a file, which the press
 * waits for and saves on this computer. The rest only send input, which waits
 * for nothing, the picture showing the effect
 * (`packages/plugin-mobile/src/index.ts`).
 */
function act(control: DeviceControl): Promise<void> | undefined {
  const call = control.call(props.data.device, { recording: recording.value })
  if (call.method === 'screenshot' || call.method === 'record') {
    return props.session.run(call).then(save)
  }
  void props.session.run(call)
  return undefined
}

/** The toolbar's actions, in the controls' order; those a narrow panel has no room for go into More. */
const actions = computed<ToolbarAction[]>(() => controls.value.map((control) => {
  const state = { recording: recording.value }
  const pressed = control.pressed?.(state) ?? false
  return {
    label: t(control.label(state)),
    icon: control.icon(state),
    group: control.group,
    pressed,
    indicator: pressed ? 'danger' : undefined,
    run: () => act(control),
  }
}))
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col">
    <Toolbar :actions="actions" :overlay-store="overlays" :more-label="t('toolbar.more')">
      <template #leading>
        <!-- The name and the model each show whole or not at all: beside the
             empty first item each fits on the one line, or wraps to the next,
             which the row cuts off, the model first, as it comes after the name.
             A phone's tab already names the device. -->
        <div class="flex h-10 min-w-0 flex-1 flex-wrap items-center overflow-hidden phone:invisible">
          <span class="h-10 w-0" />
          <span class="flex h-10 items-center gap-1.5 whitespace-nowrap pl-2 text-[13px] text-fg-body" :class="model ? 'pr-1' : 'pr-2'">
            <MobilePlatformLogo :platform="data.platform" class="text-fg-subtle" />
            {{ data.name }}
          </span>
          <span v-if="model" class="flex h-10 items-center whitespace-nowrap pr-2 text-[13px] text-fg-subtle">
            {{ t('toolbar.model', { model }) }}
          </span>
        </div>
      </template>
    </Toolbar>
    <div
      v-if="session.pictures.value === 'unsupported'"
      class="flex min-h-0 flex-1 items-center justify-center px-6 text-center text-[13px] text-fg-faint"
    >
      {{ t('region.unsupported') }}
    </div>
    <DeviceView v-else :session="session.session" :device="data.device" :shown="shown" />
  </div>
</template>
