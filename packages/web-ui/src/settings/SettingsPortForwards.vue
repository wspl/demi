<script setup lang="ts">
import { t } from './locales/text'
import { t as uiText } from '../locales/text'
import { useElementSize } from '@vueuse/core'
import { computed, nextTick, ref, watch } from 'vue'
import { ChevronRight, Plus, TriangleAlert } from '@lucide/vue'
import { deviceGlyph, hostGlyph } from '../devices/device-glyphs'
import { errorMessage, reportError } from '../infra/errors'
import type { OverlayStore } from '../overlay/overlayStore'
import AsyncRegion from '../ui/AsyncRegion.vue'
import Button from '../ui/Button.vue'
import Checkbox from '../ui/Checkbox.vue'
import CommitTextInput from '../ui/CommitTextInput.vue'
import ConfirmDialog from '../ui/ConfirmDialog.vue'
import Dropdown from '../ui/Dropdown.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import Switch from '../ui/Switch.vue'
import Tooltip from '../ui/Tooltip.vue'
import TruncatedText from '../ui/TruncatedText.vue'
import { ICON_PX } from '../ui/icon-metrics'
import type { Action } from '../ui/operations'
import { usePhone } from '../ui/phone'
import type { SentenceText } from '../ui/ui-text'
import { useListSelection } from './list-selection'
import SettingsPage from './SettingsPage.vue'
import SettingsPortForwardPage from './SettingsPortForwardPage.vue'
import {
  forwardConnect,
  forwardListen,
  forwardReason,
  forwardRoute,
  forwardState,
  forwardTone,
  type ForwardHost,
  type ForwardTone,
  type PortForwardChange,
  type SettingsPortForward,
} from './port-forwards'

/**
 * Settings › Port Forwarding (`crates/runner-protocol/src/forward.rs`): the
 * user's forwards as one table, each a row edited in place.
 *
 * ```
 * ☐  Listen On     Port    Connect To    Address          ◯
 * ☐  ● ☁ Cloud ⌄   11434   alex-mbp ⌄   localhost:11434  ●
 * ```
 *
 * A forward is its route: nothing names it, and where it is said alone, a
 * control's accessible name or the Delete dialog's list, it is said by its
 * route (`forwardRoute`).
 *
 * - A text cell reads as its value until the pointer or the focus is on it,
 *   and saves as it is left; a value it refuses keeps the cell red, with why
 *   on hover. A Host cell is the bare menu field, with the Host's mark.
 * - The dot is the forward's state, said on hover. A forward that waits or
 *   failed has its dot open what is wrong, with Try Again for a taken port,
 *   so no row grows when one fails.
 * - Checking rows turns the title's line into what to do with them: Turn On,
 *   Turn Off and Delete…, at the same height; in the list, one Actions menu
 *   holds them.
 * - Add Forward creates one at once with the backend's defaults.
 * - On a phone, and wherever the page is too narrow for the table's columns
 *   to hold a Host's name, a port and an address side by side, the forwards
 *   are a list instead: each a row of its dot, where it listens over where
 *   it connects, and its switch, which opens its own page (`shown`) in the
 *   list's place; Select checks rows.
 */
const props = defineProps<{
  load?: 'loading' | 'ready' | 'failed'
  forwards: readonly SettingsPortForward[]
  /** The user's Hosts, the Cloud's included, which a forward's ends are chosen among. */
  hosts: readonly ForwardHost[]
  /** The forward whose page the list opened; null shows the forwards. */
  shown?: string | null
  overlayStore: OverlayStore
  /** Add Forward: creates one with the defaults and answers its id, or null when it was not created. */
  onAdd?: () => Promise<string | null>
  /** Changes a forward; a refusal the user can fix, such as a port out of range, is thrown for its field to show. */
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

const phone = usePhone()
const body = ref<HTMLElement | null>(null)
const { width } = useElementSize(body)
/**
 * The page's last width, kept while a forward's page takes its place, so the
 * forwards come back as they went rather than as a table for a frame.
 */
const pageWidth = ref(0)
watch(width, (next) => {
  if (next > 0) {
    pageWidth.value = next
  }
})
/** The narrowest page, in rem, whose table shows short Host names, a port and an address in full. */
const TABLE_MIN_REM = 36
/** The forwards as the list rather than the table. */
const asList = computed(() => {
  if (phone.value) {
    return true
  }
  const rem = Number.parseFloat(getComputedStyle(document.documentElement).fontSize)
  return pageWidth.value > 0 && pageWidth.value < TABLE_MIN_REM * rem
})
const page = computed(() => (props.shown ? props.forwards.find((forward) => forward.id === props.shown) ?? null : null))

const ids = computed(() => props.forwards.map((forward) => forward.id))
const selection = useListSelection(ids)
// The rows checked stay checked across the change: the list shows them in
// its Select mode, the table by its checkboxes, which need no mode.
watch(asList, (list) => {
  selection.checking.value = list && selection.selected.value.size > 0
})
const checked = computed(() => props.forwards.filter((forward) => selection.selected.value.has(forward.id)))
const allChecked = computed(() => props.forwards.length > 0 && checked.value.length === props.forwards.length)
const someChecked = computed(() => checked.value.length > 0 && !allChecked.value)

/** The shown page's change. */
async function changeShown(change: PortForwardChange): Promise<void> {
  if (page.value) {
    await props.onChange?.(page.value.id, change)
  }
}

/** Each cell's refusal, by `forwardId/field`, while the cell keeps the refused value. */
const problems = ref<Record<string, SentenceText | undefined>>({})

/** Saves a cell's change and answers why it was refused, said as the fix, or null once saved. */
async function save(forward: SettingsPortForward, change: PortForwardChange): Promise<SentenceText | null> {
  try {
    await props.onChange?.(forward.id, change)
    return null
  } catch (error) {
    return errorMessage(error)
  }
}

/** A change no cell shows the refusal of, a switch's or a Host's: the product reports it. */
function change(forward: SettingsPortForward, next: PortForwardChange): void {
  void props.onChange?.(forward.id, next).catch(() => undefined)
}

function portProblem(value: string): SentenceText | null {
  const port = Number(value.trim())
  return /^\d+$/.test(value.trim()) && port >= 1 && port <= 65535 ? null : t('portForwards.page.portProblem')
}

function hostOf(id: string | null): ForwardHost | null {
  return id === null ? null : props.hosts.find((host) => host.id === id) ?? null
}

/** A chosen Host's glyph as its menu row draws it; a Host the account no longer lists is the plain computer. */
function glyph(id: string) {
  const host = hostOf(id)
  return host ? hostGlyph(host) : deviceGlyph(null, null)
}

const DOT: Record<ForwardTone, string> = {
  carrying: 'bg-on-success',
  idle: 'bg-fg-subtle',
  waiting: 'bg-on-warning',
  failed: 'bg-on-danger',
  off: 'bg-transparent ring-1 ring-inset ring-fg-subtle',
}

/** What a dot says on hover: the state, and why it goes through the relay when a direct path failed. */
function dotText(forward: SettingsPortForward): string {
  const reason = forwardReason(forward)
  return reason ? `${forwardState(forward, props.hosts)}. ${reason}` : forwardState(forward, props.hosts)
}

function portInUse(forward: SettingsPortForward): boolean {
  return forward.enabled && forward.state?.kind === 'portInUse'
}

const adding = ref(false)

async function add(): Promise<void> {
  adding.value = true
  try {
    const id = await props.onAdd?.()
    if (!id) {
      return
    }
    if (asList.value) {
      emit('show', id)
      return
    }
    await nextTick()
    document.querySelector<HTMLInputElement>(`[data-forward="${CSS.escape(id)}"] input`)?.focus()
  } finally {
    adding.value = false
  }
}

async function turn(enabled: boolean): Promise<void> {
  for (const forward of checked.value) {
    if (forward.enabled !== enabled) {
      try {
        await props.onChange?.(forward.id, { enabled })
      } catch {
        // The product reports it; the others go on.
      }
    }
  }
}

const deleteOpen = ref(false)
/** The forwards Delete… asks about, kept while its question closes. */
const deleting = ref<SettingsPortForward[]>([])

function askDelete(): void {
  deleting.value = checked.value
  deleteOpen.value = true
}

async function remove(): Promise<void> {
  try {
    for (const forward of deleting.value) {
      await props.onRemove?.(forward.id)
    }
  } catch (error) {
    reportError(t('portForwards.deleteFailed'), error, { site: 'portForwards.deleteFailed', userVisible: true })
  } finally {
    deleteOpen.value = false
    selection.check(false)
  }
}

/**
 * The table's columns, which every row shares as a subgrid: a Host column is
 * as wide as its longest name, Address takes the rest, and a column short of
 * room cuts its text short. The outer columns pad the rows, so a row's fill
 * reaches the table's edges.
 */
const COLUMNS = 'grid-cols-[0.25rem_1.25rem_minmax(4.5rem,auto)_3.75rem_minmax(3.5rem,auto)_minmax(6.5rem,1fr)_2.25rem_0.25rem]'
const ROW = 'col-span-full grid grid-cols-subgrid items-center border-b border-line [&>*:first-child]:col-start-2'
/** A cell typed in: its value as text at rest, a fill under the pointer and with the focus, as a cell of Numbers. */
const CELL = 'flex h-7 min-w-0 cursor-text items-center rounded-md px-2 transition-colors duration-200 ease-out hover:bg-hover focus-within:bg-hover'
</script>

<template>
  <SettingsPortForwardPage
    v-if="page"
    :key="page.id"
    :forward="page"
    :hosts="hosts"
    :overlay-store="overlayStore"
    :on-change="changeShown"
    :on-try-again="() => onTryAgain?.(page!.id)"
    :on-remove="() => onRemove?.(page!.id)"
    @back="emit('show', null)"
  />
  <SettingsPage v-else :title="t('portForwards.title')" :description="t('portForwards.description')">
    <div ref="body" class="flex flex-col gap-2">
      <!-- The title's line holds the title or, while rows are checked, what to do with them, at the same height. -->
      <div class="flex h-8 select-none items-center gap-2">
        <template v-if="checked.length || selection.checking.value">
          <span class="min-w-0 truncate whitespace-nowrap text-[15px] font-medium text-fg-emphasis">{{ t('portForwards.selected', { count: checked.length }) }}</span>
          <!-- The list's line has no room for three buttons: they are one menu, on the same line. -->
          <span v-if="asList" class="ml-auto flex shrink-0 gap-1.5">
            <Dropdown variant="default" size="sm" :overlay-store="overlayStore" :disabled="!checked.length">
              <template #trigger>{{ t('portForwards.actions') }}</template>
              <template #content="{ close }">
                <Menu>
                  <MenuItem :label="t('portForwards.turnOn')" @select="close(); turn(true)" />
                  <MenuItem :label="t('portForwards.turnOff')" @select="close(); turn(false)" />
                  <MenuItem :label="t('portForwards.delete')" is-danger @select="close(); askDelete()" />
                </Menu>
              </template>
            </Dropdown>
            <Button size="sm" variant="ghost" @click="selection.check(false)">{{ uiText('common.done') }}</Button>
          </span>
          <span v-else class="ml-auto flex shrink-0 gap-1.5">
            <Button size="sm" :disabled="!checked.length" @click="turn(true)">{{ t('portForwards.turnOn') }}</Button>
            <Button size="sm" :disabled="!checked.length" @click="turn(false)">{{ t('portForwards.turnOff') }}</Button>
            <Button size="sm" variant="danger" :disabled="!checked.length" @click="askDelete">{{ t('portForwards.delete') }}</Button>
          </span>
        </template>
        <template v-else>
          <h3 class="text-[15px] font-medium text-fg-emphasis">{{ t('portForwards.yours') }}</h3>
          <span class="ml-auto flex gap-1.5">
            <Button v-if="asList && forwards.length" size="sm" variant="ghost" @click="selection.check(true)">{{ t('portForwards.select') }}</Button>
            <Button size="sm" :loading="adding" @click="add"><Plus :size="ICON_PX.in24" /> {{ t('portForwards.add') }}</Button>
          </span>
        </template>
      </div>

      <div class="overflow-hidden rounded-xl border border-line bg-surface-float">
        <AsyncRegion :state="load" :label="t('portForwards.loading')" @retry="emit('retry')">
          <template v-if="asList">
            <div
              v-for="forward in forwards"
              :key="forward.id"
              class="flex min-h-row cursor-default items-center gap-3 border-b border-line px-4 py-2 transition-colors duration-200 ease-out last:border-b-0"
              :class="selection.selected.value.has(forward.id) ? 'bg-active' : 'hover:bg-hover'"
              @click="selection.checking.value ? selection.toggle(forward.id) : emit('show', forward.id)"
            >
              <span v-if="selection.checking.value" class="checkbox-mark shrink-0" :data-state="selection.selected.value.has(forward.id) ? 'checked' : 'unchecked'" aria-hidden="true" />
              <span class="size-2 shrink-0 rounded-full" :class="DOT[forwardTone(forward, hosts)]" :aria-label="forwardState(forward, hosts)" />
              <span class="flex min-w-0 flex-1 flex-col">
                <TruncatedText class="text-setting text-fg-emphasis tabular-nums" :text="forwardListen(forward, hosts)" />
                <TruncatedText class="text-detail text-fg-subtle tabular-nums" :text="forwardConnect(forward, hosts)" />
              </span>
              <template v-if="!selection.checking.value">
                <span class="flex items-center" @click.stop>
                  <Switch :model-value="forward.enabled" :aria-label="forwardRoute(forward, hosts)" @update:model-value="change(forward, { enabled: $event })" />
                </span>
                <ChevronRight :size="ICON_PX.in24" class="shrink-0 text-fg-subtle" />
              </template>
            </div>
          </template>

          <div v-else :class="COLUMNS" class="grid gap-x-2">
            <div :class="ROW" class="h-8 select-none text-[11px] font-medium text-fg-subtle">
              <Checkbox
                :model-value="allChecked"
                :partial="someChecked"
                :disabled="!forwards.length"
                :label="t('portForwards.selectAll')"
                class="[&>span:last-child]:sr-only"
                @update:model-value="selection.toggleAll()"
              />
              <span class="truncate pl-8">{{ t('portForwards.page.listenOn') }}</span>
              <span class="truncate pl-2">{{ t('portForwards.page.port') }}</span>
              <span class="truncate pl-2">{{ t('portForwards.page.connectTo') }}</span>
              <span class="truncate pl-2">{{ t('portForwards.page.address') }}</span>
              <span />
            </div>
            <div
              v-for="forward in forwards"
              :key="forward.id"
              :data-forward="forward.id"
              :class="[ROW, selection.selected.value.has(forward.id) ? 'bg-active' : '']"
              class="h-10 last:border-b-0"
            >
              <Checkbox
                :model-value="selection.selected.value.has(forward.id)"
                :label="t('portForwards.selectLabel', { forward: forwardRoute(forward, hosts) })"
                class="[&>span:last-child]:sr-only"
                @update:model-value="selection.toggle(forward.id, $event)"
              />
              <span class="flex min-w-0 items-center gap-2">
                <Dropdown
                  v-if="forwardTone(forward, hosts) === 'failed' || forwardTone(forward, hosts) === 'waiting'"
                  :overlay-store="overlayStore"
                  class="flex size-4 shrink-0"
                >
                  <template #trigger>
                    <span role="button" tabindex="0" :aria-label="forwardState(forward, hosts)" class="flex size-4 items-center justify-center rounded-full hover:bg-hover">
                      <span class="size-2 rounded-full" :class="DOT[forwardTone(forward, hosts)]" />
                    </span>
                  </template>
                  <template #content="{ close }">
                    <div class="flex w-64 flex-col gap-2 rounded-xl border border-line bg-surface-float p-3 shadow-lg">
                      <span class="text-[13px] font-medium text-fg-emphasis tabular-nums">{{ forwardRoute(forward, hosts) }}</span>
                      <span class="text-[13px] leading-5 text-fg-muted">{{ forwardState(forward, hosts) }}</span>
                      <Button v-if="portInUse(forward)" size="sm" class="self-start" @click="onTryAgain?.(forward.id); close()">
                        {{ t('portForwards.page.tryAgain') }}
                      </Button>
                    </div>
                  </template>
                </Dropdown>
                <Tooltip v-else :content="dotText(forward)" class="flex size-4 shrink-0 items-center justify-center">
                  <span class="size-2 rounded-full" :class="DOT[forwardTone(forward, hosts)]" :aria-label="forwardState(forward, hosts)" />
                </Tooltip>
                <Dropdown
                  :overlay-store="overlayStore"
                  variant="bare"
                  width="shrink"
                  class="min-w-0 max-w-full"
                  :trigger-label="t('portForwards.listenOnOf', { forward: forwardRoute(forward, hosts) })"
                  :icon="glyph(forward.listenDevice)"
                >
                  <template #trigger>{{ hostOf(forward.listenDevice)?.name ?? t('portForwards.unknownHost') }}</template>
                  <template #content="{ close }">
                    <Menu>
                      <MenuItem
                        v-for="host in hosts"
                        :key="host.id"
                        :icon="hostGlyph(host)"
                        :label="host.name"
                        choice
                        :is-selected="forward.listenDevice === host.id"
                        @select="close(); host.id !== forward.listenDevice && change(forward, { listenDevice: host.id })"
                      />
                    </Menu>
                  </template>
                </Dropdown>
              </span>
              <Tooltip :content="problems[`${forward.id}/port`] ?? ''" :disabled="!problems[`${forward.id}/port`]" class="flex min-w-0">
                <span :class="[CELL, problems[`${forward.id}/port`] ? 'ring-1 ring-on-danger' : '']" class="flex-1">
                  <CommitTextInput
                    bare
                    size="sm"
                    :model-value="String(forward.listenPort)"
                    :aria-label="t('portForwards.portOf', { forward: forwardRoute(forward, hosts) })"
                    :validate="portProblem"
                    :save="(value) => save(forward, { listenPort: Number(value.trim()) })"
                    inputmode="numeric"
                    trim
                    class="min-w-0 flex-1 tabular-nums"
                    @problem="problems[`${forward.id}/port`] = $event ?? undefined"
                  />
                </span>
              </Tooltip>
              <Dropdown
                :overlay-store="overlayStore"
                variant="bare"
                width="shrink"
                class="max-w-full justify-self-start"
                :trigger-label="t('portForwards.connectToOf', { forward: forwardRoute(forward, hosts) })"
                :icon="forward.connectDevice === null ? undefined : glyph(forward.connectDevice)"
              >
                <!-- Missing, the warning takes the Host's icon's place, as the forward's page shows it. -->
                <template v-if="forward.connectDevice === null" #icon>
                  <TriangleAlert :size="ICON_PX.in28" class="text-on-warning" aria-hidden="true" />
                </template>
                <template #trigger>{{ forward.connectDevice === null ? t('portForwards.page.chooseHost') : hostOf(forward.connectDevice)?.name ?? t('portForwards.unknownHost') }}</template>
                <template #content="{ close }">
                  <Menu>
                    <MenuItem
                      v-for="host in hosts"
                      :key="host.id"
                      :icon="hostGlyph(host)"
                      :label="host.name"
                      choice
                      :is-selected="forward.connectDevice === host.id"
                      @select="close(); host.id !== forward.connectDevice && change(forward, { connectDevice: host.id })"
                    />
                  </Menu>
                </template>
              </Dropdown>
              <Tooltip :content="problems[`${forward.id}/address`] ?? ''" :disabled="!problems[`${forward.id}/address`]" class="flex min-w-0">
                <span :class="[CELL, problems[`${forward.id}/address`] ? 'ring-1 ring-on-danger' : '']" class="flex-1">
                  <CommitTextInput
                    bare
                    size="sm"
                    :model-value="forward.connectAddress"
                    :aria-label="t('portForwards.addressOf', { forward: forwardRoute(forward, hosts) })"
                    placeholder="localhost:11434"
                    :save="(value) => save(forward, { connectAddress: value.trim() })"
                    literal
                    trim
                    class="min-w-0 flex-1 tabular-nums"
                    @problem="problems[`${forward.id}/address`] = $event ?? undefined"
                  />
                </span>
              </Tooltip>
              <Switch :model-value="forward.enabled" size="sm" :aria-label="forwardRoute(forward, hosts)" @update:model-value="change(forward, { enabled: $event })" />
            </div>
          </div>
          <div v-if="!forwards.length" class="select-none px-4 py-6 text-center text-[13px] text-fg-subtle">
            {{ t('portForwards.none') }}
          </div>
        </AsyncRegion>
      </div>
    </div>

    <ConfirmDialog
      :is-open="deleteOpen"
      :overlay-store="overlayStore"
      :title="t('portForwards.deleteTitle', { count: deleting.length }, deleting.length)"
      :action="uiText('common.delete')"
      :goes="deleting.map((forward) => forwardRoute(forward, hosts))"
      :operation="{ action: 'remove', target: 'port-forwards:checked' }"
      @close="deleteOpen = false"
      @confirm="remove"
    >
      <p>{{ t('portForwards.deleteBody', {}, deleting.length) }}</p>
    </ConfirmDialog>
  </SettingsPage>
</template>
