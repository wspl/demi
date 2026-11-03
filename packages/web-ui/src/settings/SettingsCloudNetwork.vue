<script setup lang="ts">
import { ref } from 'vue'
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
import { PRIVATE_NETWORK_ACCESSES, type CloudNetworkSetting, type PrivateNetworkAccess } from './cloud-network'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import { t } from './locales/text'

/**
 * Settings › Cloud Network, the master's: whether the server's Clouds reach
 * the private network, all of it or only the addresses listed.
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
  /** The setting, null until it is read. */
  network: CloudNetworkSetting | null
  load: 'loading' | 'ready' | 'failed'
  /** Why reading the setting failed. */
  loadDetail?: string | null
  overlayStore: OverlayStore
  onRetry?: () => unknown
  onSetAccess?: Action<[PrivateNetworkAccess]>
  /** Saves the whole list, answering why the server refused it, or null once saved. */
  saveDestinations?: (destinations: string[]) => Promise<SentenceText | null>
}>()

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

async function add(): Promise<void> {
  const written = draft.value.trim()
  const listed = props.network?.destinations ?? []
  if (written === '' || saving.value) {
    return
  }
  problem.value = listed.includes(written) ? null : await save([...listed, written])
  if (problem.value === null) {
    draft.value = ''
  }
}

/** Removes the row at `index`; a refusal is the setting's, not the field's, so it is a toast. */
async function remove(index: number): Promise<void> {
  const refusal = await save((props.network?.destinations ?? []).filter((_, at) => at !== index))
  if (refusal !== null) {
    showToast({ title: t('cloudNetwork.failed'), message: refusal, tone: 'danger' })
  }
}
</script>

<template>
  <SettingsPage :title="t('cloudNetwork.title')" :description="t('cloudNetwork.description')">
    <SettingsGroup>
      <AsyncRegion
        :state="network ? 'ready' : load"
        :label="t('cloudNetwork.loading')"
        :detail="loadDetail"
        :on-retry="onRetry"
      >
        <SettingsRow
          v-if="network"
          :label="t('cloudNetwork.access')"
          :description="network.access === 'entire' ? t('cloudNetwork.entireDescription') : t('cloudNetwork.accessDescription')"
        >
          <Dropdown
            size="sm"
            :overlay-store="overlayStore"
            variant="default"
            :trigger-label="t('cloudNetwork.access')"
          >
            <template #trigger>{{ t(`cloudNetwork.accesses.${network.access}`) }}</template>
            <template #content="{ close }">
              <Menu>
                <MenuItem
                  v-for="access in PRIVATE_NETWORK_ACCESSES"
                  :key="access"
                  :label="t(`cloudNetwork.accesses.${access}`)"
                  choice
                  :is-selected="network.access === access"
                  @select="close(); access === network.access || onSetAccess?.(access)"
                />
              </Menu>
            </template>
          </Dropdown>
        </SettingsRow>
      </AsyncRegion>
    </SettingsGroup>

    <SettingsGroup
      v-if="network?.access === 'only'"
      :title="t('cloudNetwork.addresses')"
      :description="t('cloudNetwork.addressesDescription')"
    >
      <!-- The field is bare text; its hit area is the whole row height. -->
      <div class="flex min-h-10 flex-col justify-center gap-1 px-3 py-1">
        <TextInput
          v-model="draft"
          size="sm"
          bare
          trim
          :placeholder="t('cloudNetwork.placeholder')"
          :aria-label="t('cloudNetwork.add')"
          class="min-w-0"
          @keydown.enter.prevent="add()"
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
        <Tooltip :content="t('cloudNetwork.remove')">
          <IconButton
            size="sm"
            :icon="Trash2"
            variant="ghost"
            :aria-label="t('cloudNetwork.remove')"
            :disabled="saving"
            @click="remove(index)"
          />
        </Tooltip>
      </SettingsRow>
      <div
        v-if="network.destinations.length === 0"
        class="flex select-none justify-center px-4 py-6 text-center text-[13px] text-fg-subtle"
      >{{ t('cloudNetwork.empty') }}</div>
    </SettingsGroup>
  </SettingsPage>
</template>
