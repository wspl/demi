<script setup lang="ts">
import { computed, defineAsyncComponent, ref, watch } from 'vue'
import { Toolbar, downloadUrl, usePage, type ToolbarAction } from '@demicodes/plugin-sdk'
import DeviceView from './DeviceView.vue'
import { drawsWebgl } from './webgl'
import type { MobileDevicesController } from './controller'
import { POSTURE_DEGREES, deviceControls, type DeviceControl } from './controls'
import { MobilePlatformLogo, type MobileTabData } from './kind'
import type { Posture } from '../generated/plugin'
import { t } from '../locales/text'

/**
 * A device's tab (`crates/plugin-mobile/page/src/index.ts`): its toolbar, and the
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
  /** The conversation's simulators, which the plugin's store keeps. */
  session: MobileDevicesController
  tabId: string
  data: MobileTabData
  shown: boolean
}>()

const { files, errors, overlays } = usePage()
/** The device's live view, on its own stream, which the conversation's simulators keep. */
const device = props.session.device(props.data)
/** Whether the device records, as the device says, whoever started it. */
const recording = computed(() => device.recording)
/**
 * Whether this panel started the recording that runs. A recording it started
 * that ends without its Stop, at the limit of its time or its size, gives it
 * the video all the same.
 */
const startedHere = ref(false)
watch(recording, (now, was) => {
  if (was && !now && startedHere.value) {
    startedHere.value = false
    void props.session.collect(props.data.device).then(save)
  }
})
/**
 * The device's model as its driver reads it: the Host's once it said, the
 * tab's until then; none when the driver knows none.
 */
const model = computed(() => device.model)
/** The controls of the device's platform, the tab's until the Host says: Back on Android alone. */
const controls = computed(() => deviceControls(device.platform ?? props.data.platform, device.fold?.kind ?? null))
/**
 * A device on a hinge shows in 3D with its bar, where the browser draws
 * WebGL 2, which three.js needs; in its lit display's 2D frame elsewhere.
 */
// three.js loads only for a device that folds on a hinge.
const HingedDevice = defineAsyncComponent(() => import('./HingedDevice.vue'))
const hinged = ref<{ posture(to: number): void } | null>(null)
const hinge = computed(() => device.fold?.kind === 'hinge' && drawsWebgl() ? device.fold : null)

/** Where the page reads the device's 3D body: the file on the Host, through its files. */
const bodyUrl = computed(() => {
  const body = hinge.value?.body
  return body ? files(props.conversation).tree?.source.contents?.url(body) ?? null : null
})

/** The file a control left on the Host goes to the user's computer, as a download does. */
function save(path: string | null): void {
  if (!path) {
    return
  }
  const contents = files(props.conversation).tree?.source.contents
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
 * (`crates/plugin-mobile/page/src/index.ts`).
 */
function act(control: DeviceControl): Promise<void> | undefined {
  const state = { recording: recording.value, fold: device.fold }
  const call = control.call(props.data.device, state)
  if (call.method === 'fold') {
    // A posture the device is in moves it there again, as a fold outside
    // Demi may have left it elsewhere; a hinged one's view moves the hinge
    // as it shows it.
    if (hinged.value && hinge.value) {
      hinged.value.posture(POSTURE_DEGREES[call.params.to as Posture])
      return undefined
    }
  }
  if (call.method === 'record') {
    // Stop takes the video itself; only a start leaves one to come.
    startedHere.value = call.params.action === 'start'
  }
  if (call.method === 'screenshot' || call.method === 'record') {
    return props.session.run(call).then(save)
  }
  void props.session.run(call)
  return undefined
}

/** The toolbar's actions, in the controls' order; those a narrow panel has no room for go into More. */
const actions = computed<ToolbarAction[]>(() => controls.value.map((control) => {
  const state = { recording: recording.value, fold: device.fold }
  const pressed = control.pressed?.(state) ?? false
  return {
    label: t(control.label(state)),
    icon: control.icon(state),
    group: control.group,
    pressed,
    indicator: pressed && control.alerts ? 'danger' : undefined,
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
    <HingedDevice
      v-else-if="hinge"
      ref="hinged"
      :session="device"
      :fold="hinge"
      :shown="shown"
      :body-url="bodyUrl"
      :downloads="session.api.downloading()"
    />
    <DeviceView v-else :session="device" :shown="shown" :downloads="session.api.downloading()" />
  </div>
</template>
