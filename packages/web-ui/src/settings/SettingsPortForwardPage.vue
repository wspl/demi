<script setup lang="ts">
import { t } from './locales/text'
import { t as uiText } from '../locales/text'
import { computed, ref } from 'vue'
import { TriangleAlert } from '@lucide/vue'
import { ICON_PX } from '../ui/icon-metrics'
import { deviceGlyph, hostGlyph } from '../devices/device-glyphs'
import Button from '../ui/Button.vue'
import CommitTextInput from '../ui/CommitTextInput.vue'
import ConfirmDialog from '../ui/ConfirmDialog.vue'
import Dropdown from '../ui/Dropdown.vue'
import InlineError from '../ui/InlineError.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import Switch from '../ui/Switch.vue'
import type { StatusDotTone } from '../ui/StatusDot.vue'
import type { SentenceText } from '../ui/ui-text'
import { shownError } from '../infra/errors'
import type { OverlayStore } from '../overlay/overlayStore'
import type { Action } from '../ui/operations'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import {
  forwardReason,
  forwardState,
  type ForwardHost,
  type PortForwardChange,
  type SettingsPortForward,
} from './port-forwards'

/**
 * A port forward's page (`crates/runner-protocol/src/forward.rs`). It shows:
 *
 * - its state,
 * - why it goes through the relay when a direct path failed, and
 * - every field, each saved as it is committed.
 *
 * Connect To shows as missing until a Host is chosen, and Remove Forward asks first.
 */
const props = defineProps<{
  forward: SettingsPortForward
  hosts: readonly ForwardHost[]
  nameMaxLength: number
  overlayStore: OverlayStore
  /** Saves a change; a refusal the user can fix is thrown, and its field shows it. */
  onChange: (change: PortForwardChange) => Promise<void>
  onTryAgain?: Action
  onRemove?: Action
}>()
const emit = defineEmits<{
  back: []
}>()

/** Each field's refusal, under the field, until it is committed again. */
const problems = ref<Partial<Record<keyof PortForwardChange, SentenceText>>>({})

async function save(field: keyof PortForwardChange, change: PortForwardChange): Promise<void> {
  try {
    await props.onChange(change)
    problems.value = { ...problems.value, [field]: undefined }
  } catch (error) {
    problems.value = { ...problems.value, [field]: shownError(error).message }
  }
}

function nameProblem(value: string): SentenceText | null {
  const name = value.trim()
  if (!name || name.length > props.nameMaxLength) {
    return t('portForwards.page.nameProblem', { max: props.nameMaxLength })
  }
  return null
}

function portProblem(value: string): SentenceText | null {
  const port = Number(value.trim())
  return /^\d+$/.test(value.trim()) && port >= 1 && port <= 65535 ? null : t('portForwards.page.portProblem')
}

const hostName = (id: string | null) =>
  id === null ? null : (props.hosts.find((host) => host.id === id)?.name ?? t('portForwards.unknownHost'))

/** A chosen Host's glyph as its menu row draws it; a Host the account no longer lists is the plain computer. */
function chosenGlyph(id: string) {
  const host = props.hosts.find((candidate) => candidate.id === id)
  return host ? hostGlyph(host) : deviceGlyph(null, null)
}

const state = computed(() => forwardState(props.forward, props.hosts))
const reason = computed(() => forwardReason(props.forward))
/** The dot before the state: green while it works, amber while it cannot, grey while off. */
const tone = computed<StatusDotTone>(() => {
  const forward = props.forward
  if (!forward.enabled) {
    return 'muted'
  }
  const offline = [forward.listenDevice, forward.connectDevice].some(
    (id) => id === null || props.hosts.find((host) => host.id === id)?.state !== 'online',
  )
  return offline || forward.state?.kind === 'portInUse' ? 'warning' : 'success'
})
const portInUse = computed(() => props.forward.enabled && props.forward.state?.kind === 'portInUse')

const removeOpen = ref(false)
const removing = computed(() => ({ action: 'remove', target: `port-forward:${props.forward.id}` }))
async function remove(): Promise<void> {
  try {
    await props.onRemove?.()
  } finally {
    removeOpen.value = false
  }
}
</script>

<template>
  <SettingsPage
    :title="forward.name"
    :back="t('portForwards.title')"
    :status-tone="tone"
    @back="emit('back')"
  >
    <template #status>{{ state }}</template>
    <template v-if="reason || portInUse" #description>
      <div class="flex min-w-0 flex-col items-start gap-2">
        <p v-if="reason">{{ reason }}</p>
        <Button v-if="portInUse" size="sm" @click="onTryAgain?.()">{{ t('portForwards.page.tryAgain') }}</Button>
      </div>
    </template>

    <SettingsGroup>
      <SettingsRow :label="t('portForwards.page.name')">
        <template v-if="problems.name" #detail>
          <InlineError :message="problems.name" />
        </template>
        <CommitTextInput
          :model-value="forward.name"
          :aria-label="t('portForwards.page.name')"
          :validate="nameProblem"
          trim
          class="w-56 max-w-full"
          @problem="problems.name = $event ?? undefined"
          @commit="save('name', { name: $event.trim() })"
        />
      </SettingsRow>
      <SettingsRow :label="t('portForwards.page.on')" :description="t('portForwards.page.onDescription')">
        <Switch
          :model-value="forward.enabled"
          :aria-label="t('portForwards.page.on')"
          size="sm"
          @update:model-value="save('enabled', { enabled: $event })"
        />
      </SettingsRow>
    </SettingsGroup>

    <SettingsGroup :title="t('portForwards.page.listenOn')">
      <SettingsRow :label="t('portForwards.page.host')" :description="t('portForwards.page.listenDescription')">
        <template v-if="problems.listenDevice" #detail>
          <InlineError :message="problems.listenDevice" />
        </template>
        <Dropdown
          :overlay-store="overlayStore"
          variant="field"
          size="sm"
          width="shrink"
          :trigger-label="t('portForwards.page.listenOn')"
          :icon="chosenGlyph(forward.listenDevice)"
        >
          <template #trigger><span class="min-w-0 truncate">{{ hostName(forward.listenDevice) }}</span></template>
          <template #content="{ close }">
            <Menu>
              <MenuItem
                v-for="host in hosts"
                :key="host.id"
                :icon="hostGlyph(host)"
                :label="host.name"
                choice
                :is-selected="forward.listenDevice === host.id"
                @select="close(); host.id !== forward.listenDevice && save('listenDevice', { listenDevice: host.id })"
              />
            </Menu>
          </template>
        </Dropdown>
      </SettingsRow>
      <SettingsRow :label="t('portForwards.page.port')">
        <template v-if="problems.listenPort" #detail>
          <InlineError :message="problems.listenPort" />
        </template>
        <CommitTextInput
          :model-value="String(forward.listenPort)"
          :aria-label="t('portForwards.page.port')"
          :validate="portProblem"
          inputmode="numeric"
          trim
          class="w-28 max-w-full"
          @problem="problems.listenPort = $event ?? undefined"
          @commit="save('listenPort', { listenPort: Number($event.trim()) })"
        />
      </SettingsRow>
    </SettingsGroup>

    <SettingsGroup :title="t('portForwards.page.connectTo')">
      <SettingsRow :label="t('portForwards.page.host')" :description="t('portForwards.page.connectDescription')">
        <!-- Missing is no failure: the menu's face says it, amber, as the page's state does. -->
        <template v-if="problems.connectDevice" #detail>
          <InlineError :message="problems.connectDevice" />
        </template>
        <Dropdown
          :overlay-store="overlayStore"
          variant="field"
          size="sm"
          width="shrink"
          :trigger-label="t('portForwards.page.connectTo')"
          :icon="forward.connectDevice === null ? undefined : chosenGlyph(forward.connectDevice)"
        >
          <!-- Missing, the warning takes the Host's icon's place. -->
          <template v-if="forward.connectDevice === null" #icon>
            <TriangleAlert :size="ICON_PX.in28" class="text-on-warning" aria-hidden="true" />
          </template>
          <template #trigger>
            <span class="min-w-0 truncate">{{ hostName(forward.connectDevice) ?? t('portForwards.page.chooseHost') }}</span>
          </template>
          <template #content="{ close }">
            <Menu>
              <MenuItem
                v-for="host in hosts"
                :key="host.id"
                :icon="hostGlyph(host)"
                :label="host.name"
                choice
                :is-selected="forward.connectDevice === host.id"
                @select="close(); host.id !== forward.connectDevice && save('connectDevice', { connectDevice: host.id })"
              />
            </Menu>
          </template>
        </Dropdown>
      </SettingsRow>
      <SettingsRow :label="t('portForwards.page.address')" :description="t('portForwards.page.addressDescription')">
        <template v-if="problems.connectAddress" #detail>
          <InlineError :message="problems.connectAddress" />
        </template>
        <CommitTextInput
          :model-value="forward.connectAddress"
          :aria-label="t('portForwards.page.address')"
          placeholder="localhost:11434"
          literal
          trim
          class="w-56 max-w-full"
          @commit="save('connectAddress', { connectAddress: $event.trim() })"
        />
      </SettingsRow>
    </SettingsGroup>

    <SettingsGroup>
      <SettingsRow :label="t('portForwards.page.removeThis')">
        <Button size="sm" variant="danger" :operation="removing" @click="removeOpen = true">{{ t('portForwards.page.remove') }}</Button>
      </SettingsRow>
    </SettingsGroup>

    <ConfirmDialog
      :is-open="removeOpen"
      :overlay-store="overlayStore"
      :title="t('portForwards.page.removeTitle', { name: forward.name })"
      :action="uiText('common.remove')"
      :operation="removing"
      :on-confirm="remove"
      @close="removeOpen = false"
    >
      <p>{{ t('portForwards.page.removeBody') }}</p>
    </ConfirmDialog>
  </SettingsPage>
</template>
