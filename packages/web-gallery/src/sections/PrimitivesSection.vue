<script setup lang="ts">
import { computed, ref } from 'vue'
import { ChevronDown, RefreshCw, Plus, Search, Send, Settings2, Trash2 } from '@lucide/vue'
import CopyButton from '@demicodes/web-ui/ui/CopyButton.vue'
import CopyCode from '@demicodes/web-ui/ui/CopyCode.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Checkbox from '@demicodes/web-ui/ui/Checkbox.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import IndeterminateSpinner from '@demicodes/web-ui/ui/IndeterminateSpinner.vue'
import Switch from '@demicodes/web-ui/ui/Switch.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import ScrollArea from '@demicodes/web-ui/ui/ScrollArea.vue'
import ShortcutRecorder from '@demicodes/web-ui/ui/ShortcutRecorder.vue'
import StatusDot, { type StatusDotTone } from '@demicodes/web-ui/ui/StatusDot.vue'
import SwatchPicker from '@demicodes/web-ui/ui/SwatchPicker.vue'
import ResizeHandle from '@demicodes/web-ui/ui/ResizeHandle.vue'
import ChoiceCards, { type ChoiceCardOption } from '@demicodes/web-ui/ui/ChoiceCards.vue'
import { Cloud, Monitor } from '@lucide/vue'
import { PRODUCT_ACCENTS } from '@demicodes/web-ui/theme/productAppearance'
import TokenInput from '@demicodes/web-ui/ui/TokenInput.vue'
import HighlightText from '@demicodes/web-ui/ui/HighlightText.vue'
import ThemeToggle from '@demicodes/web-ui/ui/ThemeToggle.vue'
import ConversationStatusDot from '@demicodes/web-ui/agent/ConversationStatusDot.vue'
import SessionDockChip from '@demicodes/web-ui/agent/SessionDockChip.vue'
import DeviceIcon from '@demicodes/web-ui/devices/DeviceIcon.vue'
import ContextUsageIndicator from '@demicodes/web-ui/agent/ContextUsageIndicator.vue'
import VendorMark from '@demicodes/web-ui/ui/VendorMark.vue'
import { providerLogo, vendorLogo } from '@demicodes/web-ui/settings/vendor-logos'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import { IN_DEVELOPMENT } from '@demicodes/web-ui/ui/disabled'
import { usageAt, usageOf } from '../fixtures/catalog'
import { DAY, HOUR, MINUTE, ahead } from '../fixtures/time'
import type { UsageLimits } from '@demicodes/web-ui/agent/context-usage'
import GallerySection from '../components/GallerySection.vue'
import GalleryOverlayWell from '../components/GalleryOverlayWell.vue'
import { productWould } from '../product-would'
import { demoInstructions, demoOpenInstruction } from '../fixtures/instructions'
import GallerySpecimen from '../components/GallerySpecimen.vue'
import { useGalleryView } from '../gallery-views'

const { view } = useGalleryView()
/** A subscription, two vendors, and a vendor with no mark, which shows its initial. */
const vendorMarks = [
  { label: 'Claude Code', src: providerLogo({ kind: 'subscription', providerType: 'claude-code', vendorId: null }) },
  { label: 'Anthropic', src: vendorLogo('anthropic') },
  { label: 'OpenAI', src: vendorLogo('openai') },
  { label: 'Ollama', src: vendorLogo('ollama') },
]
const paneWidth = ref(200)
const paneHeight = ref(96)

const spinning = ref(false)

/** A clipboard that refuses, which the failure specimen of CopyButton copies to. */
function refusedCopy(): ClipboardItem {
  return new ClipboardItem({ 'text/plain': Promise.reject(new Error('This page may not write to the clipboard.')) })
}

const dotTones: readonly StatusDotTone[] = ['accent', 'success', 'warning', 'danger', 'attention', 'muted']
const dotBreathing = ref(true)

const query = ref('session cookie')
const emptyQuery = ref('')
const secret = ref('sk-ant-api03-3f2a9c1d7e5b4a6f8c2d1e9b')
const shortcut = ref('⌘K')
const swatch = ref('blue')
const tokens = ref<number | null>(200_000)
const noTokens = ref<number | null>(null)
const enabled = ref(true)
const enabledOff = ref(false)
const enabledSmOn = ref(true)
const enabledSmOff = ref(false)
const checkboxOn = ref(true)
const choice = ref<'cloud' | 'device'>('cloud')
const choiceOptions = [
  {
    value: 'cloud',
    label: 'Cloud',
    description: 'A managed workspace, ready at once.',
    icon: Cloud
  },
  {
    value: 'device',
    label: 'Device',
    description: 'A directory on one of your devices.',
    icon: Monitor
  },
] as const satisfies readonly ChoiceCardOption<'cloud' | 'device'>[]
const checkboxOff = ref(false)
const checkboxPartialOn = ref(false)
const checkboxPartial = ref(true)
const compactIdle = ref(false)
const compactWarn = ref(false)
const compactDanger = ref(false)
const compactPinned = ref(false)
/**
 * What one card specimen keeps of its account's usage limits, as the product's
 * store does: the windows, whether a manual refresh runs and the vendor's
 * words for one that failed. A probe takes a moment and reads each window a
 * little further used; the automatic one, run when the card shows, is silent.
 */
function useCardLimits(accountLabel: string, failure: string | null = null) {
  const windows = ref<UsageLimits['windows']>([
    { id: 'five_hour', label: '5h session', used: 18, max: 100, resetsAt: ahead(3 * HOUR + 38 * MINUTE) },
    { id: 'seven_day', label: '7d all models', used: 34, max: 100, resetsAt: ahead(5 * DAY + 2 * HOUR) },
    { id: 'limit:weekly_scoped:Fable', label: '7d Fable', used: 81, max: 100, resetsAt: ahead(12 * DAY) },
  ])
  const refreshing = ref(false)
  const error = ref<string | null>(null)
  const limits = computed<UsageLimits>(() => ({
    accountLabel,
    autoRefresh: true,
    windows: windows.value,
    refreshing: refreshing.value,
    error: error.value,
  }))
  function answer(): void {
    windows.value = windows.value.map((window) => ({ ...window, used: Math.min(window.max, window.used + 1) }))
    error.value = null
  }
  function refresh(automatic: boolean): void {
    if (automatic) {
      answer()
      return
    }
    if (refreshing.value) {
      return
    }
    refreshing.value = true
    window.setTimeout(() => {
      refreshing.value = false
      if (failure) {
        error.value = failure
      } else {
        answer()
      }
    }, 1200)
  }
  return { limits, refresh }
}
const cardLong = useCardLimits('zan@example.com')
const cardTruncated = useCardLimits('a-very-long-account-name.with.many.parts@subdomain.example.com')
const cardUnlabeled = useCardLimits('')
const cardFailing = useCardLimits('zan@example.com', 'Anthropic’s usage service is unavailable.')
const buttonPressed = ref(true)
const buttonGhostPressed = ref(true)
const buttonPrimaryPressed = ref(true)
const iconPressed = ref(true)
const iconCirclePressed = ref(true)

function pulseCompact(which: 'idle' | 'warn' | 'danger' | 'pinned'): void {
  const flag = which === 'idle'
    ? compactIdle
    : which === 'warn'
    ? compactWarn
    : which === 'danger'
    ? compactDanger
    : compactPinned
  flag.value = true
  window.setTimeout(() => {
    flag.value = false
  }, 1200)
}
</script>

<template>
  <div class="space-y-8">
    <template v-if="view === 'buttons'">
      <GallerySection
        title="Button"
        note="Enabled, disabled, sizes, and pressed. Primary is the default button, filled with the accent, which Return presses in a dialog. Danger, red text on a plain face, is an action that destroys, and never the default, as macOS’s alerts show it: a confirmation’s answer (Remove, Revoke, Delete), beside a primary Cancel, or an action that then asks first. A button that refreshes, renews or restarts turns its icon one whole revolution per click, and keeps turning while its work runs, always finishing the turn it is in."
      >
        <div class="specimen-row">
          <GallerySpecimen variant="spin · full revolution">
            <Button spin-on-click><RefreshCw :size="ICON_PX.in28" />Refresh</Button>
          </GallerySpecimen>
          <GallerySpecimen variant="icon spin · full revolution">
            <IconButton
              :icon="RefreshCw"
              spin-on-click
              aria-label="Refresh example"
            />
          </GallerySpecimen>
          <GallerySpecimen variant="spin · stop after current revolution">
            <Button :spinning="spinning" @click="spinning = !spinning"><RefreshCw
                :size="ICON_PX.in28"
              />{{ spinning ? 'Stop Spinning' : 'Start Spinning' }}</Button>
          </GallerySpecimen>
          <GallerySpecimen variant="spinner · primary">
            <Button variant="primary" disabled><IndeterminateSpinner />Pairing…</Button>
          </GallerySpecimen>
          <GallerySpecimen variant="spinner · default">
            <Button><IndeterminateSpinner />Loading…</Button>
          </GallerySpecimen>
          <GallerySpecimen variant="default">
            <Button size="md">Default</Button>
          </GallerySpecimen>
          <GallerySpecimen variant="default · pressed">
            <Button
              size="md"
              :pressed="buttonPressed"
              @click="buttonPressed = !buttonPressed"
            >Default</Button>
          </GallerySpecimen>
          <GallerySpecimen variant="primary">
            <Button size="md" variant="primary" @click="productWould('Save the Changes')">Primary</Button>
          </GallerySpecimen>
          <GallerySpecimen variant="primary · pressed">
            <Button
              size="md"
              variant="primary"
              :pressed="buttonPrimaryPressed"
              @click="buttonPrimaryPressed = !buttonPrimaryPressed"
            >Primary</Button>
          </GallerySpecimen>
          <GallerySpecimen variant="danger · a confirmation’s answer">
            <Button size="md" variant="danger" @click="productWould('Remove the Provider')">Remove</Button>
          </GallerySpecimen>
          <GallerySpecimen variant="danger · loading">
            <Button size="md" variant="danger" loading>Remove</Button>
          </GallerySpecimen>
          <GallerySpecimen variant="danger · asks first">
            <Button size="md" variant="danger" @click="productWould('Ask Before Deleting the Profile')">Delete</Button>
          </GallerySpecimen>
          <GallerySpecimen variant="ghost">
            <Button size="md" variant="ghost">Ghost</Button>
          </GallerySpecimen>
          <GallerySpecimen variant="ghost · pressed">
            <Button
              size="md"
              variant="ghost"
              :pressed="buttonGhostPressed"
              @click="buttonGhostPressed = !buttonGhostPressed"
            >Ghost</Button>
          </GallerySpecimen>
          <GallerySpecimen variant="default · lg">
            <Button size="lg">Large</Button>
          </GallerySpecimen>
          <GallerySpecimen variant="primary · lg">
            <Button size="lg" variant="primary">Large</Button>
          </GallerySpecimen>
          <GallerySpecimen variant="ghost · lg">
            <Button size="lg" variant="ghost">Large</Button>
          </GallerySpecimen>
          <GallerySpecimen variant="default · sm">
            <Button size="sm">Small</Button>
          </GallerySpecimen>
          <GallerySpecimen variant="primary · sm">
            <Button size="sm" variant="primary">Small</Button>
          </GallerySpecimen>
          <GallerySpecimen variant="ghost · sm">
            <Button size="sm" variant="ghost">Small</Button>
          </GallerySpecimen>
          <GallerySpecimen variant="default · xs">
            <Button size="xs">Extra Small</Button>
          </GallerySpecimen>
          <GallerySpecimen variant="primary · xs">
            <Button size="xs" variant="primary">Extra Small</Button>
          </GallerySpecimen>
          <GallerySpecimen variant="ghost · xs">
            <Button size="xs" variant="ghost">Extra Small</Button>
          </GallerySpecimen>
          <GallerySpecimen variant="default · disabled">
            <Button disabled>Disabled</Button>
          </GallerySpecimen>
          <GallerySpecimen variant="primary · disabled">
            <Button variant="primary" disabled>Disabled</Button>
          </GallerySpecimen>
          <GallerySpecimen variant="ghost · disabled">
            <Button variant="ghost" disabled>Disabled</Button>
          </GallerySpecimen>
          <GallerySpecimen variant="danger · disabled">
            <Button variant="danger" disabled>Disabled</Button>
          </GallerySpecimen>
          <GallerySpecimen variant="disabled · reason">
            <Button disabled :disabled-reason="IN_DEVELOPMENT">In Development</Button>
          </GallerySpecimen>
        </div>
      </GallerySection>

      <GallerySection
        title="CopyCode"
        note="Shared command surface, wrapping and copy feedback."
      >
        <CopyCode code="demi-runner run --backend https://demi.example.com" />
      </GallerySection>
      <GallerySection
        title="CopyButton"
        note="The one Copy control. Pressed, it puts its text on the clipboard and shows Copied, in its icon, tip and name, for 1.5 seconds; a clipboard that refuses is told in a Could Not Copy toast."
      >
        <div class="specimen-row">
          <GallerySpecimen variant="ghost · sm">
            <CopyButton source="demi-runner --version" />
          </GallerySpecimen>
          <GallerySpecimen variant="default · md · a longer tip">
            <CopyButton
              source="https://example.com/sign-in"
              label="Copy link"
              hint="Copy the link to sign in from another browser or device"
              variant="default"
              size="md"
            />
          </GallerySpecimen>
          <GallerySpecimen variant="the clipboard refuses">
            <CopyButton :source="refusedCopy" />
          </GallerySpecimen>
        </div>
      </GallerySection>
      <GallerySection
        title="IconButton"
        note="Chip and circle. Accent is send."
      >
        <div class="specimen-row">
          <GallerySpecimen variant="default">
            <div class="flex items-center gap-1">
              <IconButton :icon="Plus" />
              <IconButton :icon="Search" />
              <IconButton :icon="Settings2" />
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="ghost">
            <div class="flex items-center gap-1">
              <IconButton :icon="Plus" variant="ghost" />
              <IconButton :icon="Search" variant="ghost" />
              <IconButton :icon="Settings2" variant="ghost" />
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="ghost · pressed">
            <IconButton
              :icon="Plus"
              variant="ghost"
              :pressed="iconPressed"
              @click="iconPressed = !iconPressed"
            />
          </GallerySpecimen>
          <GallerySpecimen variant="solid · over scrolling content">
            <IconButton
              :icon="ChevronDown"
              variant="solid"
              circle
              aria-label="Scroll to bottom"
              @click="productWould('Scroll to the Bottom')"
            />
          </GallerySpecimen>
          <GallerySpecimen variant="accent">
            <IconButton :icon="Send" variant="accent" />
          </GallerySpecimen>
          <GallerySpecimen variant="danger">
            <IconButton :icon="Trash2" variant="danger" />
          </GallerySpecimen>
          <GallerySpecimen variant="circle">
            <div class="flex items-center gap-1">
              <IconButton :icon="Plus" circle />
              <IconButton :icon="ChevronDown" circle />
              <IconButton
                :icon="Plus"
                variant="ghost"
                circle
              />
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="circle · pressed">
            <IconButton
              :icon="Plus"
              variant="ghost"
              circle
              :pressed="iconCirclePressed"
              @click="iconCirclePressed = !iconCirclePressed"
            />
          </GallerySpecimen>
          <GallerySpecimen variant="circle · accent">
            <IconButton
              :icon="Send"
              variant="accent"
              circle
            />
          </GallerySpecimen>
          <GallerySpecimen variant="circle · danger">
            <IconButton
              :icon="Trash2"
              variant="danger"
              circle
            />
          </GallerySpecimen>
          <GallerySpecimen variant="xs / sm / lg">
            <div class="flex items-center gap-1">
              <IconButton :icon="Plus" size="xs" />
              <IconButton :icon="Plus" size="sm" />
              <IconButton :icon="Plus" size="lg" />
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="disabled">
            <div class="flex items-center gap-1">
              <IconButton :icon="Plus" disabled />
              <IconButton
                :icon="Send"
                variant="accent"
                disabled
              />
              <IconButton
                :icon="Trash2"
                variant="danger"
                disabled
              />
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="disabled · reason">
            <IconButton
              :icon="Plus"
              disabled
              :disabled-reason="IN_DEVELOPMENT"
              aria-label="In development"
            />
          </GallerySpecimen>
        </div>
      </GallerySection>

      <GallerySection
        title="SessionDockChip"
        note="A chip in the conversation’s dock, over the scrolling transcript: a raised button, solid whatever runs under it, hovered and pressed in its fill as every bordered control is."
      >
        <div class="specimen-row">
          <GallerySpecimen variant="running">
            <SessionDockChip dot="accent" breathing @click="productWould('Show the Running Tasks')">3 Running</SessionDockChip>
          </GallerySpecimen>
          <GallerySpecimen variant="done">
            <SessionDockChip dot="success" @click="productWould('Show the Finished Tasks')">2 Done</SessionDockChip>
          </GallerySpecimen>
        </div>
      </GallerySection>
    </template>

    <template v-if="view === 'fields'">
      <GallerySection title="Switch" note="On and off, md and sm.">
        <div class="specimen-row">
          <GallerySpecimen variant="md · on">
            <Switch v-model="enabled" label="Available" />
          </GallerySpecimen>
          <GallerySpecimen variant="md · off">
            <Switch v-model="enabledOff" label="Available" />
          </GallerySpecimen>
          <GallerySpecimen variant="sm · on">
            <Switch
              v-model="enabledSmOn"
              size="sm"
              label="Compact"
            />
          </GallerySpecimen>
          <GallerySpecimen variant="sm · off">
            <Switch
              v-model="enabledSmOff"
              size="sm"
              label="Compact"
            />
          </GallerySpecimen>
          <GallerySpecimen variant="disabled · reason">
            <Switch
              v-model="enabled"
              disabled
              :disabled-reason="IN_DEVELOPMENT"
              label="Available"
            />
          </GallerySpecimen>
        </div>
      </GallerySection>

      <GallerySection title="Checkbox" note="On, off, and mixed.">
        <div class="specimen-row">
          <GallerySpecimen variant="checked">
            <Checkbox v-model="checkboxOn" label="Show unreads" />
          </GallerySpecimen>
          <GallerySpecimen variant="unchecked">
            <Checkbox v-model="checkboxOff" label="Show unreads" />
          </GallerySpecimen>
          <GallerySpecimen variant="partial">
            <Checkbox
              v-model="checkboxPartialOn"
              v-model:partial="checkboxPartial"
              label="Show unreads"
            />
          </GallerySpecimen>
          <GallerySpecimen variant="disabled · reason">
            <Checkbox
              v-model="checkboxOn"
              disabled
              :disabled-reason="IN_DEVELOPMENT"
              label="Show unreads"
            />
          </GallerySpecimen>
        </div>
      </GallerySection>

      <GallerySection
        title="TextInput"
        note="Empty, filled, focus, bare (no frame in any state), a secret with its eye, a suffix, and the 36px lg family for page forms."
      >
        <div class="specimen-row">
          <GallerySpecimen variant="empty">
            <TextInput v-model="emptyQuery" placeholder="Search files" />
          </GallerySpecimen>
          <GallerySpecimen variant="filled">
            <TextInput v-model="query" placeholder="Search files" />
          </GallerySpecimen>
          <GallerySpecimen variant="focused">
            <TextInput
              v-model="query"
              placeholder="Search files"
              show-focus
            />
          </GallerySpecimen>
          <GallerySpecimen variant="bare">
            <TextInput
              v-model="query"
              placeholder="Search files"
              bare
            >
              <template #prefix><Search :size="14" /></template>
            </TextInput>
          </GallerySpecimen>
          <GallerySpecimen variant="secret">
            <TextInput
              v-model="secret"
              placeholder="sk-…"
              secret
            />
          </GallerySpecimen>
          <GallerySpecimen variant="suffix">
            <TextInput v-model="query" placeholder="With suffix">
              <template #suffix>
                <Search :size="14" class="text-fg-subtle" />
              </template>
            </TextInput>
          </GallerySpecimen>
          <GallerySpecimen variant="lg">
            <TextInput
              v-model="query"
              size="lg"
              placeholder="Search files"
            />
          </GallerySpecimen>
          <GallerySpecimen variant="lg · secret">
            <TextInput
              v-model="secret"
              size="lg"
              placeholder="sk-…"
              secret
            />
          </GallerySpecimen>
        </div>
      </GallerySection>

      <GallerySection
        title="ResizeHandle"
        note="The divider that sizes a pane. Drag it (the pointer is captured, the document keeps the cursor), press the arrows on it (Shift for four steps, Home and End for the bounds), or double-click for the default. The host owns the size and clamps nothing: the handle never leaves the bounds."
      >
        <div class="specimen-row specimen-row-wide items-start">
          <GallerySpecimen variant="vertical · pane before" wide>
            <div class="flex h-40 w-[32rem] max-w-full overflow-hidden rounded-lg border border-line">
              <div
                class="flex shrink-0 items-center justify-center bg-surface-base font-mono text-[11px] text-fg-muted"
                :style="{ width: `${paneWidth}px` }"
              >{{ paneWidth }}px</div>
              <ResizeHandle v-model="paneWidth" :min="120" :max="320" :default-value="200" label="Pane width" />
              <div class="flex min-w-0 flex-1 items-center justify-center text-[11px] text-fg-faint">120–320, default 200</div>
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="horizontal · pane after">
            <div class="flex h-56 w-64 flex-col overflow-hidden rounded-lg border border-line">
              <div class="flex min-h-0 flex-1 items-center justify-center text-[11px] text-fg-faint">60–160</div>
              <ResizeHandle v-model="paneHeight" orientation="horizontal" side="end" :min="60" :max="160" :default-value="96" label="Pane height" />
              <div
                class="flex shrink-0 items-center justify-center bg-surface-base font-mono text-[11px] text-fg-muted"
                :style="{ height: `${paneHeight}px` }"
              >{{ paneHeight }}px</div>
            </div>
          </GallerySpecimen>
        </div>
      </GallerySection>

      <GallerySection
        title="SwatchPicker"
        note="Colour swatches as a radio group; the chosen one wears a ring in its own colour."
      >
        <div class="specimen-row specimen-row-wide">
          <GallerySpecimen variant="accents">
            <SwatchPicker v-model="swatch" :options="PRODUCT_ACCENTS" />
          </GallerySpecimen>
        </div>
      </GallerySection>

      <GallerySection
        title="ChoiceCards"
        note="A few exclusive choices as cards with an icon, a title and a line, for a form that branches on the answer."
      >
        <div class="specimen-stack">
          <GallerySpecimen variant="two cards" wide>
            <div class="w-96 max-w-full"><ChoiceCards
                v-model="choice"
                :options="choiceOptions"
              /></div>
          </GallerySpecimen>
        </div>
      </GallerySection>

      <GallerySection
        title="ScrollArea"
        note="Every scrolling region is a ScrollArea; a scroller another library owns (the code editor’s, a rendered document’s code block, table or equation) gets the same bar through attachScrollbars. The bar floats over the content and takes no room on any system, so padding stays even whether or not the content overflows. It shows while the pointer is over that scroller, not one around it, while it scrolls and while its thumb is dragged, then hides; a click beside the thumb moves a page. A region scrolls vertically unless it says horizontal or both, and clips the other axis. Only a strip that scrolls with no bar at all (a tab row, a segmented control, the composer) hides it."
      >
        <div class="specimen-row">
          <GallerySpecimen variant="vertical">
            <ScrollArea
              class="h-40 w-64 rounded-md ring-1 ring-line"
              viewport-class="flex flex-col gap-1 p-2"
            >
              <div
                v-for="n in 24"
                :key="n"
                class="flex h-7 shrink-0 items-center rounded-md px-2 text-chrome text-fg hover:bg-hover"
              >Row {{ n }}</div>
            </ScrollArea>
          </GallerySpecimen>
          <GallerySpecimen variant="horizontal">
            <ScrollArea
              axis="x"
              class="h-40 w-64 rounded-md ring-1 ring-line"
              viewport-class="flex gap-1 p-2"
            >
              <div
                v-for="n in 12"
                :key="n"
                class="flex w-20 shrink-0 items-center justify-center rounded-md bg-sunken text-chrome text-fg"
              >Column {{ n }}</div>
            </ScrollArea>
          </GallerySpecimen>
          <GallerySpecimen variant="both">
            <ScrollArea
              axis="both"
              class="h-40 w-64 rounded-md ring-1 ring-line"
              viewport-class="p-2"
            >
              <div class="grid w-max grid-cols-[repeat(10,5rem)] gap-1">
                <div
                  v-for="n in 100"
                  :key="n"
                  class="flex h-7 items-center justify-center rounded-md bg-sunken text-chrome text-fg"
                >Cell {{ n }}</div>
              </div>
            </ScrollArea>
          </GallerySpecimen>
          <GallerySpecimen variant="fits">
            <ScrollArea
              class="h-40 w-64 rounded-md ring-1 ring-line"
              viewport-class="flex flex-col gap-1 p-2"
            >
              <div
                v-for="n in 3"
                :key="n"
                class="flex h-7 shrink-0 items-center rounded-md px-2 text-chrome text-fg hover:bg-hover"
              >Row {{ n }}</div>
            </ScrollArea>
          </GallerySpecimen>
          <GallerySpecimen variant="nested">
            <ScrollArea
              class="h-40 w-64 rounded-md ring-1 ring-line"
              viewport-class="flex flex-col gap-2 p-2"
            >
              <div class="px-2 text-chrome text-fg-muted">Outer</div>
              <ScrollArea
                class="h-20 shrink-0 rounded-md ring-1 ring-line"
                viewport-class="flex flex-col gap-1 p-1"
              >
                <div
                  v-for="n in 8"
                  :key="n"
                  class="flex h-7 shrink-0 items-center rounded-md px-2 text-chrome text-fg hover:bg-hover"
                >Inner {{ n }}</div>
              </ScrollArea>
              <div
                v-for="n in 8"
                :key="n"
                class="flex h-7 shrink-0 items-center rounded-md px-2 text-chrome text-fg hover:bg-hover"
              >Outer {{ n }}</div>
            </ScrollArea>
          </GallerySpecimen>
        </div>
      </GallerySection>

      <GallerySection
        title="ShortcutRecorder"
        note="Caps beside a Change button; recording shows what is held, a key commits, Escape cancels."
      >
        <div class="specimen-row">
          <GallerySpecimen variant="bound">
            <ShortcutRecorder v-model="shortcut" size="sm" />
          </GallerySpecimen>
          <GallerySpecimen variant="readout">
            <div class="flex items-center gap-3">
              <ShortcutRecorder v-model="shortcut" size="sm" />
              <span class="text-[12px] text-fg-subtle">{{ shortcut }}</span>
            </div>
          </GallerySpecimen>
        </div>
      </GallerySection>

      <GallerySection
        title="TokenInput"
        note="A count in thousands or millions; the unit toggles inside the field."
      >
        <div class="specimen-row specimen-row-wide">
          <GallerySpecimen variant="empty">
            <TokenInput
              v-model="noTokens"
              placeholder="128"
              class="w-32"
            />
          </GallerySpecimen>
          <GallerySpecimen variant="filled">
            <TokenInput v-model="tokens" class="w-32" />
          </GallerySpecimen>
          <GallerySpecimen variant="readout">
            <div class="flex items-center gap-2">
              <TokenInput v-model="tokens" class="w-32" />
              <span class="text-[12px] tabular-nums text-fg-subtle">{{ tokens === null ? '—' : tokens.toLocaleString() }} tokens</span>
            </div>
          </GallerySpecimen>
        </div>
      </GallerySection>
    </template>

    <template v-if="view === 'marks'">
      <GallerySection
        title="StatusDot"
        note="The product’s one dot for a status. A running thing breathes the same everywhere (the sidebar row, a tab’s corner, the status bar, the dock’s chips): the opacity runs 0.35 to 1 and the size 0.8 to 1.15 over 2.4 seconds. Under the system’s Reduce Motion setting the dot stays still at full opacity. The ring is the settled mark that holds a dot’s place."
      >
        <div class="specimen-row">
          <GallerySpecimen v-for="tone in dotTones" :key="tone" :variant="tone">
            <StatusDot :tone="tone" :label="tone" />
          </GallerySpecimen>
          <GallerySpecimen variant="ring">
            <StatusDot tone="muted" ring label="settled" />
          </GallerySpecimen>
          <GallerySpecimen variant="xs">
            <StatusDot tone="success" size="xs" label="xs" />
          </GallerySpecimen>
          <GallerySpecimen variant="breathing">
            <div class="flex items-center gap-3">
              <StatusDot tone="accent" :breathing="dotBreathing" label="running" />
              <Switch v-model="dotBreathing" label="Breathing" size="sm" />
            </div>
          </GallerySpecimen>
        </div>
      </GallerySection>

      <GallerySection
        title="ConversationStatusDot"
        note="Idle, active, done, error, and aborted."
      >
        <div class="specimen-row">
          <GallerySpecimen variant="idle">
            <div class="relative size-6 rounded-md bg-surface-raised">
              <ConversationStatusDot status="idle" />
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="active">
            <div class="relative size-6 rounded-md bg-surface-raised">
              <ConversationStatusDot status="active" />
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="done">
            <div class="relative size-6 rounded-md bg-surface-raised">
              <ConversationStatusDot status="done" />
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="error">
            <div class="relative size-6 rounded-md bg-surface-raised">
              <ConversationStatusDot status="error" />
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="aborted">
            <div class="relative size-6 rounded-md bg-surface-raised">
              <ConversationStatusDot status="aborted" />
            </div>
          </GallerySpecimen>
        </div>
      </GallerySection>

      <GallerySection
        title="DeviceIcon"
        note="The product’s one device glyph with its state’s dot: green online, amber updating, faint offline, and no dot for a device since removed. The devices list, the host menu and the sidebar draw a device with it."
      >
        <div class="specimen-row">
          <GallerySpecimen
            v-for="specimen in ([['online', 'online'], ['updating', 'updating'], ['offline', 'offline'], ['removed', null]] as const)"
            :key="specimen[0]"
            :variant="specimen[0]"
          >
            <DeviceIcon :state="specimen[1]" />
          </GallerySpecimen>
        </div>
      </GallerySection>

      <GallerySection
        title="IndeterminateSpinner"
        note="14px in-progress mark."
      >
        <div class="specimen-row">
          <GallerySpecimen variant="14">
            <IndeterminateSpinner />
          </GallerySpecimen>
        </div>
      </GallerySection>

      <GallerySection
        title="ContextUsageIndicator"
        note="The ring is the backend’s estimate. Click it for the card, on a phone tap it for a sheet; a click outside, another click on the ring or Escape closes it, and resting the pointer on it shows only its tip, Context window. Compact closes the card, as a menu’s action does. The header opens and closes the list of what fills the context, closed at first and then as the user left it, kept in the browser; the bar shows either way, the rows largest first, each one figure with its share of the window. Under them, the room before compaction, with Compact, from half the window. Then the usage limits of a subscription account, under a line that ends in a Refresh usage button (its icon turns while the request runs, the windows stay until the answer replaces them, a failure shows the vendor’s words under the line until the next answer, and a click keeps the card open), each window’s name, reset and use on one line over its bar (a model on an API key has none), and last the instruction files, each of which opens where it is written."
      >
        <div class="specimen-row">
          <GallerySpecimen variant="below half · 34%">
            <ContextUsageIndicator
              :usage="usageAt(0.34)"
              :is-compacting="compactIdle"
              @compact="pulseCompact('idle')"
            />
          </GallerySpecimen>
          <GallerySpecimen variant="warning · 75%">
            <ContextUsageIndicator
              :usage="usageAt(0.75)"
              :is-compacting="compactWarn"
              @compact="pulseCompact('warn')"
            />
          </GallerySpecimen>
          <GallerySpecimen variant="danger · 94%">
            <ContextUsageIndicator
              :usage="usageAt(0.94)"
              :is-compacting="compactDanger"
              @compact="pulseCompact('danger')"
            />
          </GallerySpecimen>
          <GallerySpecimen variant="turn running">
            <ContextUsageIndicator
              :usage="usageAt(0.75)"
              unavailable-reason="Compaction is available once the turn ends."
            />
          </GallerySpecimen>
          <GallerySpecimen variant="compacting">
            <ContextUsageIndicator
              :usage="usageAt(0.75)"
              :is-compacting="true"
            />
          </GallerySpecimen>
          <GallerySpecimen variant="no window">
            <ContextUsageIndicator
              :usage="usageOf(12_000, null)"
              @compact="productWould('Compact the Conversation')"
            />
          </GallerySpecimen>
        </div>
        <!-- Each card pinned open in a well of its own, with room above the ring for it. -->
        <div class="flex flex-wrap gap-6">
          <GalleryOverlayWell>
            <GallerySpecimen variant="card · a long conversation, Compact offered">
              <div class="px-44 pt-[36rem]">
                <ContextUsageIndicator
                  pinned
                  :usage="usageAt(0.62)"
                  :is-compacting="compactPinned"
                  :instructions="demoInstructions"
                  :limits="cardLong.limits.value"
                  @compact="pulseCompact('pinned')"
                  @open-instruction="demoOpenInstruction($event, productWould)"
                  @refresh-limits="cardLong.refresh"
                />
              </div>
            </GallerySpecimen>
          </GalleryOverlayWell>
          <GalleryOverlayWell>
            <GallerySpecimen variant="card · just compacted, a long account label truncates">
              <div class="px-44 pt-[36rem]">
                <ContextUsageIndicator
                  pinned
                  :usage="usageAt(0.06, { summary: 2_400 })"
                  :instructions="demoInstructions"
                  :limits="cardTruncated.limits.value"
                  @open-instruction="demoOpenInstruction($event, productWould)"
                  @refresh-limits="cardTruncated.refresh"
                />
              </div>
            </GallerySpecimen>
          </GalleryOverlayWell>
          <GalleryOverlayWell>
            <GallerySpecimen variant="card · account without a label">
              <div class="px-44 pt-[36rem]">
                <ContextUsageIndicator
                  pinned
                  :usage="usageAt(0.42)"
                  :instructions="demoInstructions"
                  :limits="cardUnlabeled.limits.value"
                  @compact="productWould('Compact the Conversation')"
                  @open-instruction="demoOpenInstruction($event, productWould)"
                  @refresh-limits="cardUnlabeled.refresh"
                />
              </div>
            </GallerySpecimen>
          </GalleryOverlayWell>
          <GalleryOverlayWell>
            <GallerySpecimen variant="card · Refresh usage fails, the vendor’s words show under the header">
              <div class="px-44 pt-[36rem]">
                <ContextUsageIndicator
                  pinned
                  :usage="usageAt(0.42)"
                  :instructions="demoInstructions"
                  :limits="cardFailing.limits.value"
                  @compact="productWould('Compact the Conversation')"
                  @open-instruction="demoOpenInstruction($event, productWould)"
                  @refresh-limits="cardFailing.refresh"
                />
              </div>
            </GallerySpecimen>
          </GalleryOverlayWell>
          <GalleryOverlayWell>
            <GallerySpecimen variant="card · API key, no usage limits">
              <div class="px-44 pt-[26rem]">
                <ContextUsageIndicator
                  pinned
                  :usage="usageAt(0.42)"
                  :instructions="demoInstructions"
                  @compact="productWould('Compact the Conversation')"
                  @open-instruction="demoOpenInstruction($event, productWould)"
                />
              </div>
            </GallerySpecimen>
          </GalleryOverlayWell>
        </div>
      </GallerySection>

      <GallerySection
        title="VendorMark"
        note="A vendor’s mark in the text’s color, from the one map of marks (vendor-logos): a subscription wears its vendor’s, Claude Code Claude’s own; a vendor without a mark shows its initial. Small in lists, medium beside a title."
      >
        <div class="specimen-row">
          <GallerySpecimen v-for="mark in vendorMarks" :key="mark.label" :variant="mark.label">
            <div class="flex items-center gap-2">
              <VendorMark :label="mark.label" :src="mark.src" size="sm" />
              <VendorMark :label="mark.label" :src="mark.src" />
            </div>
          </GallerySpecimen>
        </div>
      </GallerySection>

      <GallerySection title="ThemeToggle" note="Light and dark.">
        <div class="specimen-row">
          <GallerySpecimen>
            <ThemeToggle />
          </GallerySpecimen>
        </div>
      </GallerySection>

      <GallerySection
        title="HighlightText"
        note="Match, multi-match, none, and empty query."
      >
        <div class="specimen-row">
          <GallerySpecimen variant="match">
            <div class="text-[13px] text-fg-body">
              <HighlightText
                text="session cookie after the rename"
                query="session"
              />
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="multi-match">
            <div class="text-[13px] text-fg-body">
              <HighlightText
                text="session cookie, session header"
                query="session"
              />
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="no match">
            <div class="text-[13px] text-fg-body">
              <HighlightText
                text="session cookie after the rename"
                query="sid"
              />
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="empty query">
            <div class="text-[13px] text-fg-body">
              <HighlightText text="session cookie after the rename" query="" />
            </div>
          </GallerySpecimen>
        </div>
      </GallerySection>
    </template>
  </div>
</template>
