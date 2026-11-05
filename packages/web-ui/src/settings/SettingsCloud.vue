<script setup lang="ts">
import { computed, ref } from 'vue'
import { Plus, Trash2 } from '@lucide/vue'
import { formatByteRatio, formatBytes } from '../files/format'
import { showToast } from '../infra/toast'
import type { OverlayStore } from '../overlay/overlayStore'
import AsyncRegion from '../ui/AsyncRegion.vue'
import Button from '../ui/Button.vue'
import Dropdown from '../ui/Dropdown.vue'
import IconButton from '../ui/IconButton.vue'
import IndeterminateSpinner from '../ui/IndeterminateSpinner.vue'
import InlineError from '../ui/InlineError.vue'
import Menu from '../ui/Menu.vue'
import MenuDivider from '../ui/MenuDivider.vue'
import MenuItem from '../ui/MenuItem.vue'
import ProgressBar from '../ui/ProgressBar.vue'
import Switch from '../ui/Switch.vue'
import TextInput from '../ui/TextInput.vue'
import Tooltip from '../ui/Tooltip.vue'
import TruncatedText from '../ui/TruncatedText.vue'
import { ICON_PX } from '../ui/icon-metrics'
import type { Action } from '../ui/operations'
import type { SentenceText } from '../ui/ui-text'
import {
  COMMON_NETWORK_POLICIES,
  ISOLATIONS,
  NETWORK_GROUPS,
  type CloudIsolationSetting,
  type CloudNetworkSetting,
  type Isolation,
  type NetworkGroup,
  type NetworkPolicy,
} from './cloud'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import { t } from './locales/text'

/**
 * Settings › Cloud, the master's: how the server's Clouds are isolated, and
 * the network policy, which decides every destination they reach: one of the
 * common policies, or Custom, whose switches and list say what it allows.
 *
 * An isolation the server can't run stays in the menu, disabled, with the
 * server's reason as its tooltip. A setting the chosen isolation doesn't
 * enforce stays visible, disabled, and says why in place of its
 * description, as the network policy does on Simple Isolation.
 *
 * gVisor on a server that has never run it is offered with a hint that
 * choosing it downloads it. While the server downloads it and starts it, a
 * row under Isolation says how far it is, as a provider's command-line tool
 * says while it installs; a download that failed says why, with Try Again,
 * which chooses it again.
 *
 * Custom's list is a card of plain rows under one bare field, as the models
 * of a provider are under their filter: Enter in the field adds what it
 * holds, and each row has a remove button. An entry is changed by removing
 * it and adding it again, as an entry of macOS's list of DNS servers is.
 * What the server refuses stays in the field, with why under it.
 *
 * One change is saved at a time: each sends the whole list, so a second sent
 * before the first's answer would put back what the first removed.
 */
const props = defineProps<{
  /** The isolation, null until it is read. */
  isolation: CloudIsolationSetting | null
  /** The network policy, null until it is read. */
  network: CloudNetworkSetting | null
  load: 'loading' | 'ready' | 'failed'
  /** Why reading the settings failed. */
  loadDetail?: string | null
  overlayStore: OverlayStore
  onRetry?: () => unknown
  onSetIsolation?: Action<[Isolation]>
  onSetPolicy?: Action<[NetworkPolicy]>
  onSetGroup?: Action<[NetworkGroup, boolean]>
  /** Saves Custom's whole list, answering why the server refused it, or null once saved. */
  saveAllowed?: (allowed: string[]) => Promise<SentenceText | null>
}>()

/** Why the server can't run `isolation`; null when it can or hasn't said. */
function unavailable(isolation: Isolation): string | null {
  return props.isolation?.runtimes?.find((runtime) => runtime.isolation === isolation)?.unavailable ?? null
}

/** Whether choosing `isolation` downloads its release first. */
function downloads(isolation: Isolation): boolean {
  const state = props.isolation?.runtimes?.find((runtime) => runtime.isolation === isolation)?.download?.state
  return state === 'waiting' || state === 'failed'
}

/** The download of the chosen isolation's release while the server makes it or after it failed; null otherwise. */
const download = computed(() => {
  const chosen = props.isolation?.runtimes?.find((runtime) => runtime.isolation === props.isolation?.isolation)
  return chosen?.download && chosen.download.state !== 'waiting' ? chosen.download : null
})

/**
 * What the download says in place of the isolation's description, on one
 * line, so the row keeps its height in every state.
 */
const downloadStatus = computed(() => {
  const now = download.value
  switch (now?.state) {
    case 'downloading':
      return t('cloud.download.received', {
        bytes: now.total !== null ? formatByteRatio(now.received, now.total) : formatBytes(now.received),
      })
    case 'starting':
      return t('cloud.download.starting')
    case 'failed':
      return t('cloud.download.failed', { message: now.message })
    default:
      return null
  }
})

/** Whether the network policy applies to the chosen isolation; it does while the server hasn't said. */
const networkApplies = computed(() => {
  const chosen = props.isolation?.runtimes?.find((runtime) => runtime.isolation === props.isolation?.isolation)
  return chosen?.networkPolicy ?? true
})

/** What Developer Services' row says: its first domains by their brands, then how many more. */
const developerServicesDetail = computed(() =>
  t('cloud.groupDescriptions.developerServices', {
    count: Math.max((props.network?.developerServices.length ?? 0) - 3, 0),
  }),
)

const draft = ref('')
const saving = ref(false)
/** Why the server refused the address in the field. */
const problem = ref<SentenceText | null>(null)

/** Saves `next`, answering why the server refused it, or null once saved. */
async function save(next: string[]): Promise<SentenceText | null> {
  saving.value = true
  try {
    return (await props.saveAllowed?.(next)) ?? null
  } finally {
    saving.value = false
  }
}

/**
 * Adds every entry the field holds: one typed, or several pasted or typed apart by
 * lines, spaces or commas, as a list copied from a document or a firewall's rules
 * comes. One save carries them all, so the server refuses or takes them together.
 */
async function add(): Promise<void> {
  const listed = props.network?.custom.allowed ?? []
  const written = [...new Set(draft.value.split(/[\s,]+/).filter(Boolean))].filter((address) => !listed.includes(address))
  if (saving.value || draft.value.trim() === '') {
    return
  }
  problem.value = written.length ? await save([...listed, ...written]) : null
  if (problem.value === null) {
    draft.value = ''
  }
}

/** A paste of several entries adds them at once; a single one stays in the field to edit. */
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
  const refusal = await save((props.network?.custom.allowed ?? []).filter((_, at) => at !== index))
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
            :description="downloadStatus ? undefined : t(`cloud.isolationDescriptions.${isolation.isolation}`)"
          >
            <template v-if="downloadStatus" #description>
              <TruncatedText class="tabular-nums" :text="downloadStatus" />
            </template>
            <Button v-if="download?.state === 'failed'" size="sm" @click="onSetIsolation?.(isolation.isolation)">{{
              t('cloud.download.tryAgain')
            }}</Button>
            <div v-else-if="download" class="flex w-24 items-center justify-end">
              <ProgressBar
                v-if="download.state === 'downloading' && download.total"
                :value="download.received"
                :max="download.total"
                :label="t('cloud.download.downloading')"
              />
              <IndeterminateSpinner v-else />
            </div>
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
                    :hint="downloads(choice) ? t('cloud.download.hint') : undefined"
                    @select="close(); choice === isolation.isolation || onSetIsolation?.(choice)"
                  />
                </Menu>
              </template>
            </Dropdown>
          </SettingsRow>
          <SettingsRow
            :label="t('cloud.policy')"
            :description="networkApplies ? t(`cloud.policyDescriptions.${network.policy}`) : t('cloud.noNetworkPolicy')"
          >
            <Dropdown
              size="sm"
              :overlay-store="overlayStore"
              variant="default"
              :trigger-label="t('cloud.policy')"
              :disabled="!networkApplies"
              :disabled-reason="t('cloud.noNetworkPolicy')"
            >
              <template #trigger>{{ t(`cloud.policies.${network.policy}`) }}</template>
              <template #content="{ close }">
                <Menu>
                  <MenuItem
                    v-for="policy in COMMON_NETWORK_POLICIES"
                    :key="policy"
                    :label="t(`cloud.policies.${policy}`)"
                    choice
                    :is-selected="network.policy === policy"
                    @select="close(); policy === network.policy || onSetPolicy?.(policy)"
                  />
                  <MenuDivider />
                  <MenuItem
                    :label="t('cloud.policies.custom')"
                    choice
                    :is-selected="network.policy === 'custom'"
                    @select="close(); network.policy === 'custom' || onSetPolicy?.('custom')"
                  />
                </Menu>
              </template>
            </Dropdown>
          </SettingsRow>
        </template>
      </AsyncRegion>
    </SettingsGroup>

    <SettingsGroup
      v-if="network?.policy === 'custom'"
      :title="t('cloud.allowedGroups')"
      :description="t('cloud.neverReachable')"
    >
      <SettingsRow
        v-for="group in NETWORK_GROUPS"
        :key="group"
        :label="t(`cloud.groups.${group}`)"
        :description="group === 'developerServices' ? developerServicesDetail : t(`cloud.groupDescriptions.${group}`)"
      >
        <Switch
          size="sm"
          :model-value="network.custom[group]"
          :aria-label="t(`cloud.groups.${group}`)"
          :disabled="!networkApplies"
          :disabled-reason="t('cloud.noNetworkPolicy')"
          @update:model-value="onSetGroup?.(group, $event)"
        />
      </SettingsRow>
    </SettingsGroup>

    <SettingsGroup
      v-if="network?.policy === 'custom'"
      :title="t('cloud.domainsAndAddresses')"
      :description="t('cloud.domainsAndAddressesDescription')"
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
          :disabled-reason="t('cloud.noNetworkPolicy')"
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
        v-for="(entry, index) in network.custom.allowed"
        :key="entry"
        :label="entry"
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
        v-if="network.custom.allowed.length === 0"
        class="flex select-none justify-center px-4 py-6 text-center text-[13px] text-fg-subtle"
      >{{ t('cloud.empty') }}</div>
    </SettingsGroup>
  </SettingsPage>
</template>
