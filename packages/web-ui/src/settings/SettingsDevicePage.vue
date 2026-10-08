<script setup lang="ts">
import { computed, ref } from 'vue'
import Button from '../ui/Button.vue'
import Fold from '../ui/Fold.vue'
import FoldChevron from '../ui/FoldChevron.vue'
import HelpPopover from '../ui/HelpPopover.vue'
import RelativeTime from '../ui/RelativeTime.vue'
import Switch from '../ui/Switch.vue'
import { formatDay, formatWhen, useTimeUntil } from '../composables/useRelativeTime'
import DeviceRenameDialog from '../devices/DeviceRenameDialog.vue'
import DeviceRevokeDialog from '../devices/DeviceRevokeDialog.vue'
import DeviceStartHint from '../devices/DeviceStartHint.vue'
import {
  DIRECT_STAGE_LABEL,
  directReason,
  reasonSentence,
  type DirectAddresses,
} from '../devices/direct'
import { RUNNER_STATE_LABEL, runnerState, systemName, type DeviceReport } from '../devices/report'
import { DEVICE_STATE_LABEL } from '../devices/state'
import type { OverlayStore } from '../overlay/overlayStore'
import type { SentenceText } from '../ui/ui-text'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import type { SettingsDevice } from './types'

/**
 * A device's own page (`direct-channel.md` § What the user sees), which its
 * row in Devices opens: how this page reaches it and why, with the switch
 * for direct connections, Try Now and what the last attempt saw; then the
 * device itself, its system by name, its runner as up to date or not, and
 * when it was paired, with Rename… and Revoke…. The Cloud's page has no
 * switch: the Cloud is always reached through the server.
 */
const props = defineProps<{
  /** A paired device, or the Cloud with what its runner reported. */
  page: { kind: 'device'; device: SettingsDevice } | { kind: 'cloud'; report: DeviceReport }
  /** The runner release the server's devices follow; null on a server without them. */
  runnerRelease: string | null
  /** The direct channel's round trip while it stands, in milliseconds. */
  roundTripMs?: number | null
  /** The names of the projects on the device, which go with it. */
  projects?: readonly string[]
  nameMaxLength: number | null
  /** A change of the device's switch is under way. */
  changing?: boolean
  renaming?: boolean
  revoking?: boolean
  overlayStore: OverlayStore
}>()
const emit = defineEmits<{
  back: []
  setDirect: [enabled: boolean]
  tryNow: []
  rename: [name: string]
  revoke: []
}>()

const device = computed(() => (props.page.kind === 'device' ? props.page.device : null))
const report = computed<DeviceReport>(() => (props.page.kind === 'device' ? props.page.device : props.page.report))
const system = computed(() => systemName(report.value))
const runner = computed(() => runnerState(report.value.runnerVersion, props.runnerRelease))

const direct = computed(() => device.value?.direct ?? null)
const reason = computed(() => (direct.value ? directReason(direct.value) : null))
const online = computed(() => device.value?.state === 'online')
const attempt = computed(() => direct.value?.attempt ?? null)

/** The status row's words: how this page reaches the device now. */
const statusLabel = computed<SentenceText>(() => {
  const shown = device.value
  if (!shown) {
    return 'Through the server'
  }
  if (shown.state !== 'online') {
    return DEVICE_STATE_LABEL[shown.state]
  }
  return reason.value === null ? 'Connected directly' : 'Through the server'
})

const roundTrip = computed(() => {
  const ms = props.roundTripMs
  if (ms === null || ms === undefined) {
    return null
  }
  if (ms < 0.1) {
    return 'under 0.1 ms'
  }
  return ms < 10 ? `${ms.toFixed(1)} ms` : `${Math.round(ms)} ms`
})

const nextIn = useTimeUntil(() => direct.value?.nextAt ?? new Date().toISOString())

/** How long an attempt took, as a person reads it. */
function took(ms: number): string {
  return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(1)} s`
}

/** One side's addresses on one line, or that it offered none. */
function addresses(side: DirectAddresses, kind: keyof DirectAddresses): string {
  return side[kind].length ? side[kind].join(', ') : 'None'
}

const PERMISSION_LABEL = {
  granted: 'Allowed',
  prompt: 'Not answered yet',
  denied: 'Blocked',
} as const

const detailsOpen = ref(false)
const renameOpen = ref(false)
const revokeOpen = ref(false)
function revoke() {
  revokeOpen.value = false
  emit('revoke')
}
</script>

<template>
  <SettingsPage
    :title="device?.name ?? 'Cloud'"
    back="Devices"
    @back="emit('back')"
  >
    <SettingsGroup title="Connection">
      <template v-if="device">
        <SettingsRow
          label="Connect directly when possible"
          description="Files and a browser’s live view go straight to the device when the network allows it."
        >
          <Switch
            :model-value="device.direct.enabled"
            aria-label="Connect directly when possible"
            :disabled="changing"
            @update:model-value="emit('setDirect', $event)"
          />
        </SettingsRow>
        <SettingsRow :label="statusLabel">
          <template #description>
            <template v-if="device.state === 'offline'">
              <template v-if="device.seen">Last seen <RelativeTime :timestamp="device.seen" />.</template>
              <template v-else>Its runner has not connected yet.</template>
            </template>
            <template v-else-if="device.state === 'updating'">Its runner is updating itself and connects again in a moment.</template>
            <template v-else-if="reason === null">
              {{ roundTrip ? `Round trip ${roundTrip}.` : 'Files and the live view go straight to the device.' }}
            </template>
            <template v-else>{{ reasonSentence(reason, formatWhen) }}</template>
          </template>
          <HelpPopover
            v-if="device.state === 'offline' && device.start"
            label="How to Start Its Runner"
            :overlay-store="overlayStore"
          >
            <DeviceStartHint :start="device.start" />
          </HelpPopover>
        </SettingsRow>
        <SettingsRow v-if="online && device.direct.enabled" label="Last attempt">
          <template #description>
            <template v-if="device.direct.trying">Trying now…</template>
            <template v-else-if="attempt">
              Ran <RelativeTime :timestamp="attempt.startedAt" /> and took {{ took(attempt.durationMs) }}.<template
                v-if="device.direct.nextAt"
              > The next one runs {{ nextIn }}.</template>
            </template>
            <template v-else>None yet.</template>
          </template>
          <Button
            size="sm"
            :loading="device.direct.trying"
            :disabled="device.direct.connected || device.direct.permission === 'denied'"
            :disabled-reason="device.direct.connected ? 'Already connected directly.' : 'This browser blocks local network access.'"
            @click="emit('tryNow')"
          >Try Now</Button>
        </SettingsRow>
        <template v-if="attempt">
          <SettingsRow label="Details" interactive @click="detailsOpen = !detailsOpen">
            <template #accessory><FoldChevron :open="detailsOpen" /></template>
          </SettingsRow>
          <!-- The details read as the Details row's own, with no line between them. -->
          <Fold :open="detailsOpen" class="border-t-0!">
            <dl class="grid select-text grid-cols-[max-content_minmax(0,1fr)] gap-x-6 gap-y-1.5 px-4 pb-4 pt-0.5 text-[12px] leading-4">
              <dt class="text-fg-subtle">Result</dt>
              <dd class="text-fg-muted">
                {{ attempt.outcome === 'failed' && attempt.stage ? `Stopped at ${DIRECT_STAGE_LABEL[attempt.stage].toLowerCase()}` : attempt.outcome === 'busy' ? 'The device was busy' : attempt.outcome === 'dropped' ? 'Connected, then dropped' : 'Connected' }}
              </dd>
              <dt class="text-fg-subtle">Took</dt>
              <dd class="text-fg-muted">{{ took(attempt.durationMs) }}</dd>
              <dt class="text-fg-subtle">This browser</dt>
              <dd class="break-words text-fg-muted">
                Local: {{ addresses(attempt.browser, 'local') }}<br>Public: {{ addresses(attempt.browser, 'public') }}
              </dd>
              <dt class="text-fg-subtle">The device</dt>
              <dd class="break-words text-fg-muted">
                Local: {{ addresses(attempt.device, 'local') }}<br>Public: {{ addresses(attempt.device, 'public') }}
              </dd>
              <dt class="text-fg-subtle">Paths tried</dt>
              <dd class="text-fg-muted">{{ attempt.pairs.tried }}, {{ attempt.pairs.answered }} answered</dd>
              <dt class="text-fg-subtle">Local network access</dt>
              <dd class="text-fg-muted">{{ attempt.permission ? PERMISSION_LABEL[attempt.permission] : 'Not asked by this browser' }}</dd>
            </dl>
          </Fold>
        </template>
      </template>
      <SettingsRow
        v-else
        label="Through the server"
        description="The Cloud runs beside the server, so this browser always reaches it through the server."
      />
    </SettingsGroup>

    <SettingsGroup title="Device">
      <SettingsRow v-if="device" label="Name">
        <span class="min-w-0 truncate text-chrome text-fg-muted">{{ device.name }}</span>
        <Button size="sm" :loading="renaming" @click="renameOpen = true">Rename…</Button>
      </SettingsRow>
      <SettingsRow label="System">
        <span class="text-chrome text-fg-muted">{{ system ?? 'Not reported yet' }}</span>
      </SettingsRow>
      <SettingsRow
        label="Runner"
        :description="runner === 'outdated' ? 'It updates itself the next time it connects.' : undefined"
      >
        <span class="text-chrome text-fg-muted">{{ runner ? RUNNER_STATE_LABEL[runner] : 'Not reported yet' }}</span>
      </SettingsRow>
      <SettingsRow v-if="device" label="Paired">
        <span class="text-chrome text-fg-muted">{{ formatDay(device.pairedAt) }}</span>
      </SettingsRow>
    </SettingsGroup>

    <SettingsGroup v-if="device">
      <SettingsRow
        label="Revoke this device"
        description="It leaves your devices, and its runner removes itself if it is connected."
      >
        <Button size="sm" variant="danger" :loading="revoking" @click="revokeOpen = true">Revoke…</Button>
      </SettingsRow>
    </SettingsGroup>

    <DeviceRenameDialog
      :is-open="renameOpen"
      :overlay-store="overlayStore"
      :name="device?.name ?? ''"
      :max-length="nameMaxLength"
      @close="renameOpen = false"
      @rename="emit('rename', $event)"
    />
    <DeviceRevokeDialog
      :is-open="revokeOpen"
      :overlay-store="overlayStore"
      :device="device?.name ?? ''"
      :projects="projects ?? []"
      @close="revokeOpen = false"
      @revoke="revoke"
    />
  </SettingsPage>
</template>
