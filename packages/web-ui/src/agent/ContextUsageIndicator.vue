<script setup lang="ts">
import { t } from './locales/text'
import { computed, ref } from 'vue'
import { FileText, Network, RefreshCw, UserRound } from '@lucide/vue'
import type { ContextRowKind, ContextUsage, InstructionEntry } from '@demicodes/protocol'
import ProviderQuota from '../settings/ProviderQuota.vue'
import Button from '../ui/Button.vue'
import Dropdown from '../ui/Dropdown.vue'
import Fold from '../ui/Fold.vue'
import FoldChevron from '../ui/FoldChevron.vue'
import { ICON_PX, useIconPx } from '../ui/icon-metrics'
import IconButton from '../ui/IconButton.vue'
import IndeterminateSpinner from '../ui/IndeterminateSpinner.vue'
import ScrollArea from '../ui/ScrollArea.vue'
import SectionRow from '../ui/SectionRow.vue'
import Tooltip from '../ui/Tooltip.vue'
import TruncatedText from '../ui/TruncatedText.vue'
import { appOverlayStore } from '../overlay/appOverlay'
import { useLayerElevation } from '../overlay/layerElevation'
import { usePhone } from '../ui/phone'
import { formatTokens } from '../ui/token-count'
import {
  type ContextCardRow,
  type ContextCardSection,
  type UsageLimits,
  compactionRefusal,
  contextCardParts,
  contextCardRows,
  contextFigure,
  contextTip,
  contextFilled,
  contextPercent,
  contextRatio,
  formatShare,
  untilCompaction,
  useContextCardSection,
} from './context-usage'
import { instructionRows } from './instructions'
import ContextCost from './ContextCost.vue'
import { type CostState, hasCost } from './context-cost'

/**
 * The composer's context meter: a ring of how full the next request is, as
 * the backend estimates it. A click on it opens a card, on a phone a sheet
 * (`compaction.md` § What fills the context), as the model selector's does;
 * a click outside, another click on the ring or Escape closes it, and
 * resting the pointer on the ring opens nothing but its tip, Context and the figure the
 * card's header shows. The card
 * holds the estimate against the window, a header that opens and closes the list of rows, a
 * bar of what fills the context, the rows largest first while the list is
 * open, and the room before compaction with Compact; then what the
 * conversation cost, whose caption opens and closes its details
 * (`usage-and-quota.md` § A conversation's cost); then the usage limits
 * of the conversation's account, and last the instructions the model holds
 * (`instructions.md` § What the card lists), each of which opens where it
 * is written. One part stands open at most: opening one closes the other.
 * While the usage or the window is unknown the card holds the cost and the
 * usage limits, and with no limits there is no meter, cost or none.
 */
const props = defineProps<{
  usage?: ContextUsage | null
  isCompacting?: boolean
  /** Why Compact cannot run now apart from the usage, such as a running turn. */
  unavailableReason?: string | null
  /** The unfinished turn failed for its size, so Compact runs at any share (`refusedForSize`). */
  sizeRefused?: boolean
  /** The card shows without a click, as a gallery specimen pins it open. */
  pinned?: boolean
  /** What the newest instructions block holds, in its order; the card lists nothing of them while it holds none. */
  instructions?: readonly InstructionEntry[]
  /** The usage limits of the account the conversation's model runs on; none for a model on an API key. */
  limits?: UsageLimits | null
  /**
   * The meter stands as a segment of a status bar (`StatusBarSegment`): its
   * hover background fills the bar's height inside its inset and the segment's
   * width, not a rounded square, and Tab reaches it with the focus ring.
   */
  segment?: boolean
  /**
   * The conversation's cost, which the host holds while the conversation
   * is open, read with it and kept current by its frames, or the failure
   * of its read; none for a card that shows no conversation's cost.
   */
  cost?: CostState | null
  /** The part that stands open, or none, whatever the browser keeps, as a gallery specimen pins it. */
  expanded?: ContextCardSection | null
  /** The cost chart's call, by its place among the conversation's, whose tip shows without the pointer, as a gallery specimen pins it. */
  costTip?: number | null
  /** The names of the profiles the conversation's workflow offers now, which the cost's Profiles part shows. */
  costProfiles?: readonly string[]
}>()

const emit = defineEmits<{
  compact: []
  /** Opens where an entry is written: the settings for the personal instructions, the file for a project file. */
  openInstruction: [entry: InstructionEntry]
  /**
   * The host asks the vendor for the usage limits again, as Settings does:
   * automatic when the limits showed, otherwise the Refresh usage button.
   */
  refreshLimits: [automatic: boolean]
  /** Read the cost again after a read that failed. */
  retryCost: []
}>()

/** Whether the card shows the context's part, the usage limits, or neither, when the meter is not there. */
const parts = computed(() => contextCardParts(props.usage ?? null, props.limits ?? null, props.isCompacting === true))
const rows = computed<ContextCardRow[]>(() => (parts.value.context && props.usage ? contextCardRows(props.usage) : []))
const instructionList = computed(() => (parts.value.context ? instructionRows(props.instructions ?? []) : []))
const filled = computed(() => (props.usage ? contextFilled(props.usage) : 0))
/** What the bar and the shares count against: the window, or what fills a context that has none. */
const whole = computed(() => props.usage?.window ?? filled.value)
const room = computed(() => (props.usage ? untilCompaction(props.usage) : null))

/** On a phone the card is a sheet, which scrolls the details itself. */
const phone = usePhone()
/** The ring is a toolbar button's icon, and in a status bar a chip's, at the layout's size. */
const icons = useIconPx()
const ringPx = computed(() => (props.segment ? icons.value.chip : icons.value.in28))

/** The part that stands open: a header opens it and closes the other, and the browser keeps the choice. */
const expanded = props.expanded === undefined ? useContextCardSection() : ref(props.expanded)
function toggle(section: ContextCardSection): void {
  expanded.value = expanded.value === section ? null : section
}

/** Whether the card shows: a click opens it; a pinned specimen stays open whatever closes it. */
const clicked = ref(false)
const open = computed({
  get: () => props.pinned === true || clicked.value,
  set: (value: boolean) => {
    if (!props.pinned)
      clicked.value = value
  },
})
const elevation = useLayerElevation()
/**
 * The Cost section shows once the conversation made a call, or once a read
 * of it failed, with the context's part or without it.
 */
const costShown = computed(() => {
  const cost = props.cost
  if (!cost)
    return false
  return cost.status === 'failed' || hasCost(cost.cost)
})

/** Compact is an action of the card, which closes it as a menu's action does. */
function compact(close: () => void): void {
  close()
  emit('compact')
}

function openEntry(entry: InstructionEntry, close: () => void): void {
  close()
  emit('openInstruction', entry)
}

/** Each row's color in the bar and beside its name; the buffer and the free space are grays. */
const ROW_COLORS: Record<ContextRowKind, string> = {
  messages: 'bg-series-1',
  systemPrompt: 'bg-series-2',
  commands: 'bg-series-3',
  instructions: 'bg-series-4',
  skills: 'bg-series-5',
  otherContext: 'bg-series-6',
  summary: 'bg-series-7',
  compactionBuffer: 'bg-series-reserved',
  freeSpace: 'bg-overlay/8',
}

/** The bar's segments: every row but the free space, which is the track left over. */
const segments = computed(() => rows.value.filter((row) => row.kind !== 'freeSpace' && whole.value > 0))
function width(tokens: number): string {
  return `${Math.min(100, (tokens / whole.value) * 100)}%`
}

function tokensLabel(tokens: number, estimated: boolean): string {
  return estimated ? `~${formatTokens(tokens)}` : formatTokens(tokens)
}

const percent = computed(() => contextPercent(props.usage ?? null))
const ratio = computed(() => contextRatio(props.usage ?? null) ?? 0)
const blockedReason = computed(() => props.unavailableReason ?? compactionRefusal(props.usage ?? null, props.sizeRefused))
/** While compacting the card holds no Compact; it has details while anything else is left to show. */
const hasDetails = computed(() => (parts.value.context && !props.isCompacting) || rows.value.length > 0 || costShown.value || parts.value.limits || instructionList.value.length > 0)

/** The ring's tip is Context and the card header's figure; without a usage or a window the ring has none. */
const tip = computed(() => contextTip(props.usage ?? null))
/** The ring's name for a screen reader, which the tip does not carry. */
const label = computed(() => (percent.value == null ? t('context.window') : t('context.windowUsed', { percent: percent.value })))

const radius = 5.5
const circumference = 2 * Math.PI * radius
const strokeDashoffset = computed(() => circumference * (1 - ratio.value))

const ringColor = computed(() => {
  if (percent.value == null)
    return 'text-fg-subtle'
  if (ratio.value >= 0.9)
    return 'text-on-danger'
  if (ratio.value >= 0.7)
    return 'text-on-warning'
  return 'text-on-accent'
})
</script>

<template>
  <Tooltip v-if="parts.context || parts.limits" :content="tip" tag="div" class="flex">
    <Dropdown v-model:open="open" :overlay-store="appOverlayStore" side="top" :offset="8">
      <template #trigger="{ isOpen }">
        <button
          data-context-ring
          type="button"
          :aria-label="label"
          aria-haspopup="dialog"
          :aria-expanded="isOpen"
          class="relative flex cursor-default items-center justify-center transition-colors hover:bg-hover"
          :class="[segment ? 'status-bar-segment h-full' : 'size-hit rounded-md', isOpen ? 'bg-hover' : '']"
          :data-acting="segment || undefined"
          data-drawn-target
        >
          <IndeterminateSpinner v-if="isCompacting" :size="ringPx" />
          <svg
            v-else
            :width="ringPx"
            :height="ringPx"
            viewBox="0 0 14 14"
            class="-rotate-90"
          >
            <circle
              cx="7" cy="7" :r="radius"
              fill="none"
              stroke="currentColor"
              stroke-width="2.5"
              class="text-overlay/15"
            />
            <!-- A gauge, not a spinner: square ends and a track as visible as the
                 fill's room, and no arc until there is one to read, since a short
                 round-capped arc on a faint ring is what a spinner looks like. -->
            <circle
              v-if="ratio >= 0.01"
              cx="7" cy="7" :r="radius"
              fill="none"
              stroke="currentColor"
              stroke-width="2.5"
              :stroke-dasharray="String(circumference)"
              :stroke-dashoffset="strokeDashoffset"
              :class="ringColor"
            />
          </svg>
        </button>
      </template>
      <template #content="{ close }">
        <!-- On a phone the card is a sheet's content, read at a sheet's size. -->
        <div data-context-card class="overlay-shell rounded-md text-detail leading-4 text-fg phone:text-[15px] phone:leading-5" :style="{ '--elevation': elevation }">
          <div v-if="parts.context" class="context-card-head">
            <!-- The header is the list's disclosure, as a Finder disclosure row is;
                 compacting, while there are rows to list. -->
            <button
              v-if="!isCompacting || rows.length"
              type="button"
              :aria-expanded="expanded === 'context'"
              class="flex w-full min-w-0 cursor-default items-baseline gap-3 whitespace-nowrap rounded-sm text-left phone:min-h-row phone:items-center"
              @click="toggle('context')"
            >
              <span v-if="isCompacting" class="min-w-0 truncate text-fg-body">{{ t('context.compacting') }}</span>
              <template v-else-if="usage">
                <!-- The name gives way on a narrow card; the figure and the chevron stay whole. -->
                <span class="min-w-0 truncate font-medium text-fg-emphasis">{{ t('context.window') }}</span>
                <span class="ml-auto shrink-0 tabular-nums text-fg-muted">
                  {{ contextFigure(usage) }}
                </span>
              </template>
              <FoldChevron :open="expanded === 'context'" :size="ICON_PX.in20" class="self-center text-fg-subtle" :class="isCompacting ? 'ml-auto' : ''" />
            </button>
            <span v-else-if="isCompacting" class="text-fg-body">{{ t('context.compacting') }}</span>
          </div>
          <div v-if="hasDetails">
            <!-- The details scroll when the card would be taller than the room the
                 page leaves it, as with a long list of instruction files. -->
            <div class="w-[22rem] max-w-full phone:w-full" :class="phone ? '' : 'context-details-bounded'">
              <ScrollArea class="min-h-0 flex-1">
                <div v-if="rows.length" class="px-3 pt-0.5">
                  <div class="flex h-2 gap-px overflow-hidden rounded-full bg-overlay/8" aria-hidden="true">
                    <div
                      v-for="row in segments"
                      :key="row.kind"
                      class="h-full shrink-0 first:rounded-l-full"
                      :class="ROW_COLORS[row.kind]"
                      :style="{ width: width(row.tokens) }"
                    />
                  </div>
                  <!-- The list opens and closes by height, as every disclosure does (Fold). -->
                  <Fold :open="expanded === 'context'">
                    <ul class="pt-2">
                      <li v-for="row in rows" :key="row.kind" class="context-row">
                        <span class="size-2 rounded-[2px]" :class="ROW_COLORS[row.kind]" />
                        <span class="min-w-0 truncate text-fg-body">{{ row.label }}</span>
                        <span class="text-right tabular-nums text-fg-body">{{ tokensLabel(row.tokens, row.estimated) }}</span>
                        <span class="text-right tabular-nums text-fg-subtle">{{ usage?.window ? formatShare(row.tokens, usage.window) : '' }}</span>
                      </li>
                    </ul>
                  </Fold>
                </div>
                <!-- The last line of the context section: its Compact ends where the
                     figures above it end. -->
                <div v-if="parts.context && !isCompacting" class="flex items-center gap-3 px-3 pt-2 pb-2.5">
                  <span class="min-w-0 flex-1 truncate tabular-nums text-fg-muted">{{ room === null ? '' : t('context.untilCompaction', { tokens: formatTokens(room) }) }}</span>
                  <Button
                    size="sm"
                    :disabled="blockedReason != null"
                    :disabled-reason="blockedReason ?? undefined"
                    @click="compact(close)"
                  >
                    {{ t('context.compact') }}
                  </Button>
                </div>
                <ContextCost
                  v-if="costShown && cost"
                  :state="cost"
                  :open="expanded === 'cost'"
                  :first="!parts.context"
                  :pinned-tip="costTip"
                  :profiles="costProfiles"
                  @retry="emit('retryCost')"
                  @toggle="toggle('cost')"
                />
                <div v-if="limits" class="pb-3" :class="parts.context || costShown ? 'border-t border-line' : 'pt-1'">
                  <SectionRow class="text-caption font-medium text-fg-subtle">
                    <span class="shrink-0">{{ t('context.usageLimits') }}</span>
                    <template v-if="limits.accountLabel">
                      <span class="shrink-0 px-1">·</span>
                      <TruncatedText :text="limits.accountLabel" />
                    </template>
                    <template #end>
                      <!-- The click keeps the card open: it only asks, and the answer replaces the windows. -->
                      <Tooltip :content="t('context.refreshUsage')">
                        <IconButton
                          size="xs"
                          variant="ghost"
                          :icon="RefreshCw"
                          :aria-label="t('context.refreshUsage')"
                          :spinning="limits.refreshing === true"
                          :disabled="limits.refreshing === true"
                          @click="emit('refreshLimits', false)"
                        />
                      </Tooltip>
                    </template>
                  </SectionRow>
                  <div class="px-3 pt-0.5">
                    <ProviderQuota
                      :windows="limits.windows"
                      :error="limits.error"
                      :auto-refresh="limits.autoRefresh"
                      @refresh="emit('refreshLimits', true)"
                    />
                  </div>
                </div>
                <div v-if="instructionList.length" class="border-t border-line pb-1">
                  <div class="px-3 pt-2 pb-1 text-caption font-medium text-fg-subtle">{{ t('context.instructions') }}</div>
                  <ul class="px-1">
                    <li v-for="row in instructionList" :key="row.path ?? 'personal'">
                      <Tooltip tag="div" :content="row.path ?? undefined">
                        <button
                          type="button"
                          class="flex w-full items-center gap-2 rounded px-2 py-1 text-left hover:bg-hover phone:min-h-row phone:py-0"
                          @click="openEntry(row.entry, close)"
                        >
                          <UserRound v-if="row.entry.kind === 'personal'" :size="icons.chip" class="shrink-0 text-fg-subtle" />
                          <Network v-else-if="row.entry.kind === 'workflow'" :size="icons.chip" class="shrink-0 text-fg-subtle" />
                          <FileText v-else :size="icons.chip" class="shrink-0 text-fg-subtle" />
                          <span class="min-w-0 flex-1 truncate text-fg-body">{{ row.label }}</span>
                        </button>
                      </Tooltip>
                    </li>
                  </ul>
                </div>
              </ScrollArea>
            </div>
          </div>
        </div>
      </template>
    </Dropdown>
  </Tooltip>
</template>

<style scoped>
/* Beside the trigger the details take the room the page leaves the card,
   less its header row, and scroll in it; a phone's sheet scrolls them
   itself. */
.context-details-bounded {
  display: flex;
  flex-direction: column;
  max-height: calc(var(--overlay-available-height, 80vh) - 2.25rem);
}

/* The card's header row: its text, one line high, 2.25rem at least. */
.context-card-head {
  display: flex;
  align-items: center;
  min-height: 2.25rem;
  padding: 0.5rem 0.75rem;
  line-height: 1.25rem;
}

/* On a phone the disclosure is a row's height itself, the head no taller. */
@media (max-width: 767px) {
  .context-card-head {
    padding-block: 0;
  }
}

/* A row: its color, its name, its tokens and its share of the window. */
.context-row {
  display: grid;
  grid-template-columns: 0.5rem minmax(0, 1fr) auto 3rem;
  align-items: center;
  column-gap: 0.5rem;
  min-height: 1.5rem;
}
</style>
