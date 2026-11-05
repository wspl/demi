<script setup lang="ts">
import { computed, ref } from 'vue'
import { Plus, Trash2 } from '@lucide/vue'
import { showToast } from '../infra/toast'
import type { OverlayStore } from '../overlay/overlayStore'
import AsyncRegion from '../ui/AsyncRegion.vue'
import Dropdown from '../ui/Dropdown.vue'
import IconButton from '../ui/IconButton.vue'
import InlineError from '../ui/InlineError.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import TextInput from '../ui/TextInput.vue'
import Tooltip from '../ui/Tooltip.vue'
import { ICON_PX } from '../ui/icon-metrics'
import type { Action } from '../ui/operations'
import type { SentenceText } from '../ui/ui-text'
import {
  ISOLATIONS,
  PRIVATE_NETWORK_ACCESSES,
  type CloudIsolationSetting,
  type CloudNetworkSetting,
  type Isolation,
  type PrivateNetworkAccess,
} from './cloud'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import { t } from './locales/text'

/**
 * Settings › Cloud, the master's: how the server's Clouds are isolated, and
 * whether they reach the private network, all of it or only the addresses
 * listed.
 *
 * An isolation the server can't run stays in the menu, disabled, with the
 * server's reason as its tooltip. A setting the chosen isolation doesn't
 * enforce stays visible, disabled, and says why in place of its
 * description, as Private Network Access does on Simple Isolation.
 *
 * The list is a card of plain rows under one bare field, as the models of a
 * provider are under their filter: Enter in the field adds what it holds,
 * and each row has a remove button. An address is changed by removing it and
 * adding it again, as an entry of macOS's list of DNS servers is. What the
 * server refuses stays in the field, with why under it.
 *
 * One change is saved at a time: each sends the whole list, so a second sent
 * before the first's answer would put back what the first removed.
 */
const props = defineProps<{
  /** The isolation, null until it is read. */
  isolation: CloudIsolationSetting | null
  /** Private Network Access, null until it is read. */
  network: CloudNetworkSetting | null
  load: 'loading' | 'ready' | 'failed'
  /** Why reading the settings failed. */
  loadDetail?: string | null
  overlayStore: OverlayStore
  onRetry?: () => unknown
  onSetIsolation?: Action<[Isolation]>
  onSetAccess?: Action<[PrivateNetworkAccess]>
  /** Saves the whole list, answering why the server refused it, or null once saved. */
  saveDestinations?: (destinations: string[]) => Promise<SentenceText | null>
}>()

/** Why the server can't run `isolation`; null when it can or hasn't said. */
function unavailable(isolation: Isolation): string | null {
  return props.isolation?.runtimes?.find((runtime) => runtime.isolation === isolation)?.unavailable ?? null
}

/** Whether Private Network Access applies to the chosen isolation; it does while the server hasn't said. */
const networkApplies = computed(() => {
  const chosen = props.isolation?.runtimes?.find((runtime) => runtime.isolation === props.isolation?.isolation)
  return chosen?.privateNetwork ?? true
})

const draft = ref('')
const saving = ref(false)
/** Why the server refused the address in the field. */
const problem = ref<SentenceText | null>(null)

/** Saves `next`, answering why the server refused it, or null once saved. */
async function save(next: string[]): Promise<SentenceText | null> {
  saving.value = true
  try {
    return (await props.saveDestinations?.(next)) ?? null
  } finally {
    saving.value = false
  }
}

/**
 * Adds every address the field holds: one typed, or several pasted or typed apart by
 * lines, spaces or commas, as a list copied from a document or a firewall's rules
 * comes. One save carries them all, so the server refuses or takes them together.
 */
async function add(): Promise<void> {
  const listed = props.network?.destinations ?? []
  const written = [...new Set(draft.value.split(/[\s,]+/).filter(Boolean))].filter((address) => !listed.includes(address))
  if (saving.value || draft.value.trim() === '') {
    return
  }
  problem.value = written.length ? await save([...listed, ...written]) : null
  if (problem.value === null) {
    draft.value = ''
  }
}

/** A paste of several addresses adds them at once; a single one stays in the field to edit. */
function paste(event: ClipboardEvent): void {
  const text = event.clipboardData?.getData('text') ?? ''
  if (text.trim().split(/[\s,]+/).length < 2) {
    return
  }
  event.preventDefault()
  draft.value = text.trim().split(/[\s,]+/).join(', ')
  void add()
}

/** Removes the row at `index`; a refusal is the setting's, not the field's, so it is a toast. */
async function remove(index: number): Promise<void> {
  const refusal = await save((props.network?.destinations ?? []).filter((_, at) => at !== index))
  if (refusal !== null) {
    showToast({ title: t('cloud.failed'), message: refusal, tone: 'danger' })
  }
}
</script>

<template>
  <SettingsPage :title="t('cloud.title')" :description="t('cloud.description')">
    <SettingsGroup>
      <AsyncRegion
        :state="network && isolation ? 'ready' : load"
        :label="t('cloud.loading')"
        :detail="loadDetail"
        :on-retry="onRetry"
      >
        <template v-if="network && isolation">
          <SettingsRow
            :label="t('cloud.isolation')"
            :description="t(`cloud.isolationDescriptions.${isolation.isolation}`)"
          >
            <Dropdown
              size="sm"
              :overlay-store="overlayStore"
              variant="default"
              :trigger-label="t('cloud.isolation')"
            >
              <template #trigger>{{ t(`cloud.isolations.${isolation.isolation}`) }}</template>
              <template #content="{ close }">
                <Menu>
                  <MenuItem
                    v-for="choice in ISOLATIONS"
                    :key="choice"
                    :label="t(`cloud.isolations.${choice}`)"
                    choice
                    :is-selected="isolation.isolation === choice"
                    :disabled="unavailable(choice) !== null"
                    :disabled-reason="t('cloud.unavailable', { reason: unavailable(choice) ?? '' })"
                    @select="close(); choice === isolation.isolation || onSetIsolation?.(choice)"
                  />
                </Menu>
              </template>
            </Dropdown>
          </SettingsRow>
          <SettingsRow
            :label="t('cloud.access')"
            :description="
              !networkApplies
                ? t('cloud.noPrivateNetwork')
                : network.access === 'entire'
                  ? t('cloud.entireDescription')
                  : t('cloud.accessDescription')
            "
          >
            <Dropdown
              size="sm"
              :overlay-store="overlayStore"
              variant="default"
              :trigger-label="t('cloud.access')"
              :disabled="!networkApplies"
              :disabled-reason="t('cloud.noPrivateNetwork')"
            >
              <template #trigger>{{ t(`cloud.accesses.${network.access}`) }}</template>
              <template #content="{ close }">
                <Menu>
                  <MenuItem
                    v-for="access in PRIVATE_NETWORK_ACCESSES"
                    :key="access"
                    :label="t(`cloud.accesses.${access}`)"
                    choice
                    :is-selected="network.access === access"
                    @select="close(); access === network.access || onSetAccess?.(access)"
                  />
                </Menu>
              </template>
            </Dropdown>
          </SettingsRow>
        </template>
      </AsyncRegion>
    </SettingsGroup>

    <SettingsGroup
      v-if="network?.access === 'only'"
      :title="t('cloud.addresses')"
      :description="t('cloud.addressesDescription')"
    >
      <!-- The field is bare text; its hit area is the whole row height. -->
      <div class="flex min-h-10 flex-col justify-center gap-1 px-3 py-1">
        <TextInput
          v-model="draft"
          size="sm"
          bare
          trim
          :placeholder="t('cloud.placeholder')"
          :aria-label="t('cloud.add')"
          :disabled="!networkApplies"
          :disabled-reason="t('cloud.noPrivateNetwork')"
          class="min-w-0"
          @keydown.enter.prevent="add()"
          @paste="paste"
          @update:model-value="problem = null"
        >
          <template #prefix><Plus :size="ICON_PX.in24" /></template>
        </TextInput>
        <InlineError v-if="problem" :message="problem" class="pb-1" />
      </div>
      <SettingsRow
        v-for="(destination, index) in network.destinations"
        :key="destination"
        :label="destination"
        compact
      >
        <Tooltip :content="t('cloud.remove')">
          <IconButton
            size="sm"
            :icon="Trash2"
            variant="ghost"
            :aria-label="t('cloud.remove')"
            :disabled="saving || !networkApplies"
            @click="remove(index)"
          />
        </Tooltip>
      </SettingsRow>
      <div
        v-if="network.destinations.length === 0"
        class="flex select-none justify-center px-4 py-6 text-center text-[13px] text-fg-subtle"
      >{{ t('cloud.empty') }}</div>
    </SettingsGroup>
  </SettingsPage>
</template>
