<script setup lang="ts">
import { t } from './locales/text'
import { computed } from 'vue'
import AsyncRegion from '../ui/AsyncRegion.vue'
import Button from '../ui/Button.vue'
import Switch from '../ui/Switch.vue'
import type { OverlayStore } from '../overlay/overlayStore'
import type { Action } from '../ui/operations'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsPortForwardPage from './SettingsPortForwardPage.vue'
import SettingsRow from './SettingsRow.vue'
import {
  forwardRoute,
  forwardState,
  type ForwardHost,
  type PortForwardChange,
  type SettingsPortForward,
} from './port-forwards'

/**
 * Settings › Port Forwarding (`crates/runner-protocol/src/forward.rs`): the user's
 * forwards, one row each with its ends, its switch and its state.
 *
 * Each row opens its own page, which `shown` names, in the list's place. Add Forward
 * creates one at once and opens its page.
 */
const props = defineProps<{
  load?: 'loading' | 'ready' | 'failed'
  forwards: readonly SettingsPortForward[]
  /** The user's Hosts, the Cloud's included, which a forward's ends are chosen among. */
  hosts: readonly ForwardHost[]
  /** The forward whose page is shown; null shows the list. */
  shown?: string | null
  /** The most characters a forward's name has. */
  nameMaxLength: number
  overlayStore: OverlayStore
  /** Add Forward: creates one with the defaults; its page opens once it exists. */
  onAdd?: Action
  /** Changes a forward; a refusal the user can fix, such as a taken name, is thrown for its field to show. */
  onChange?: (id: string, change: PortForwardChange) => Promise<void>
  /** Try Again on a taken port: the Listen On runner tries to listen again. */
  onTryAgain?: Action<[id: string]>
  /** Remove Forward, once the user confirmed it. */
  onRemove?: Action<[id: string]>
}>()
const emit = defineEmits<{
  retry: []
  /** Shows a forward's page, or the list again with null. */
  show: [id: string | null]
}>()

const page = computed(() => (props.shown ? props.forwards.find((forward) => forward.id === props.shown) ?? null : null))

/** The shown page's change. */
async function changeShown(change: PortForwardChange): Promise<void> {
  if (page.value)
    await props.onChange?.(page.value.id, change)
}

/** The switch of a row: its forward's change, whose failure the product reports. */
function turn(forward: SettingsPortForward, enabled: boolean): Promise<void> | undefined {
  return props.onChange?.(forward.id, { enabled }).catch(() => undefined)
}
</script>

<template>
  <SettingsPortForwardPage
    v-if="page"
    :key="page.id"
    :forward="page"
    :hosts="hosts"
    :name-max-length="nameMaxLength"
    :overlay-store="overlayStore"
    :on-change="changeShown"
    :on-try-again="() => onTryAgain?.(page!.id)"
    :on-remove="() => onRemove?.(page!.id)"
    @back="emit('show', null)"
  />
  <SettingsPage
    v-else
    :title="t('portForwards.title')"
    :description="t('portForwards.description')"
  >
    <SettingsGroup :title="t('portForwards.yours')">
      <template #actions>
        <Button size="sm" @click="onAdd?.()">{{ t('portForwards.add') }}</Button>
      </template>
      <AsyncRegion
        :state="load"
        :label="t('portForwards.loading')"
        @retry="emit('retry')"
      >
        <SettingsRow
          v-for="forward in forwards"
          :key="forward.id"
          :label="forward.name"
          :target="`port-forward:${forward.id}`"
          navigable
          isolate-controls
          @click="emit('show', forward.id)"
        >
          <template #description>
            <span class="block">{{ forwardRoute(forward, hosts) }}</span>
            <span class="block">{{ forwardState(forward, hosts) }}</span>
          </template>
          <Switch
            :model-value="forward.enabled"
            :aria-label="forward.name"
            size="sm"
            @update:model-value="turn(forward, $event)"
          />
        </SettingsRow>
        <div
          v-if="!forwards.length"
          class="select-none px-4 py-6 text-center text-[13px] text-fg-subtle"
        >
          {{ t('portForwards.none') }}
        </div>
      </AsyncRegion>
    </SettingsGroup>
  </SettingsPage>
</template>
