<script setup lang="ts">
import { computed, ref } from 'vue'
import { ChevronDown, RefreshCw, Plus, Search, Send, Settings2, Trash2 } from '@lucide/vue'
import CopyButton from '@demicodes/web-ui/ui/CopyButton.vue'
import CopyCode from '@demicodes/web-ui/ui/CopyCode.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Checkbox from '@demicodes/web-ui/ui/Checkbox.vue'
import ExternalLink from '@demicodes/web-ui/ui/ExternalLink.vue'
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
import { productAccentOptions } from '@demicodes/web-ui/theme/productAppearance'
import TokenInput from '@demicodes/web-ui/ui/TokenInput.vue'
import HighlightText from '@demicodes/web-ui/ui/HighlightText.vue'
import ThemeToggle from '@demicodes/web-ui/ui/ThemeToggle.vue'
import ConversationStatusDot from '@demicodes/web-ui/agent/ConversationStatusDot.vue'
import SessionDockChip from '@demicodes/web-ui/agent/SessionDockChip.vue'
import DeviceIcon from '@demicodes/web-ui/devices/DeviceIcon.vue'
import type { DeviceLogo, DeviceMark } from '@demicodes/web-ui/devices/mark'
import type { DeviceState } from '@demicodes/web-ui/devices/state'
import ContextUsageIndicator from '@demicodes/web-ui/agent/ContextUsageIndicator.vue'
import VendorMark from '@demicodes/web-ui/ui/VendorMark.vue'
import Avatar from '@demicodes/web-ui/ui/Avatar.vue'
import SettingsGroup from '@demicodes/web-ui/settings/SettingsGroup.vue'
import SettingsRow from '@demicodes/web-ui/settings/SettingsRow.vue'
import { HUE_TONES, hueToneOf } from '@demicodes/web-ui/ui/avatar-tone'
import { accountDisplayName } from '@demicodes/web-ui/auth/account-display'
import { providerLogo, vendorLogo } from '@demicodes/web-ui/settings/vendor-logos'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import { inDevelopment } from '@demicodes/web-ui/ui/disabled'
import { focusedByKeyboard } from '@demicodes/web-ui/ui/active-row'
import { usageAt, usageOf } from '../fixtures/catalog'
import { useCardLimits } from '../fixtures/usage-limits'
import { MANY_PROFILES, OFFERED_PROFILES, useCardCost } from '../fixtures/cost'
import GallerySection from '../components/GallerySection.vue'
import GalleryOverlayWell from '../components/GalleryOverlayWell.vue'
import GalleryWorkingControls from '../components/GalleryWorkingControls.vue'
import { productWould } from '../product-would'
import { demoInstructions, demoOpenInstruction } from '../fixtures/instructions'
import GallerySpecimen from '../components/GallerySpecimen.vue'
import { useGalleryView } from '../gallery-views'

const { view } = useGalleryView()
/**
 * A subscription, two vendors with models.dev's marks, one with a mark kept
 * by hand, and a vendor with no mark, which shows its initial.
 */
const vendorMarks = [
  { label: 'Claude Code', src: providerLogo({ kind: 'subscription', providerType: 'claude-code', vendorId: null }) },
  { label: 'Anthropic', src: vendorLogo('anthropic') },
  { label: 'Volcengine', src: vendorLogo('volcengine') },
  { label: 'BytePlus', src: vendorLogo('byteplus') },
  { label: 'Ollama', src: vendorLogo('ollama') },
]
/** The accounts the Avatar specimens draw: two words, one word, one CJK character, only an email, two letters. */
const avatarAccounts = [
  { id: 'acc-1', name: 'Alex Morgan', email: 'alex@example.com' },
  { id: 'acc-2', name: 'Priya', email: 'priya@example.com' },
  { id: 'acc-3', name: '李雷', email: 'lei@example.com' },
  { id: 'acc-4', name: '', email: 'o\u2019neil@example.com' },
  { id: 'acc-5', name: 'Bo', email: 'bo@example.com' },
] as const
const avatarSelf = avatarAccounts[0]
/** One account id for each of the eight tones, found by trying ids in turn, with the tone's name as the letters. */
const avatarPalette = HUE_TONES.map((tone) => {
  for (let n = 0; ; n++) {
    if (hueToneOf(`tone-${n}`) === tone) return { name: tone.name, id: `tone-${n}` }
  }
})
const paneWidth = ref(200)
const paneHeight = ref(96)

const spinning = ref(false)
/** The list specimen of the focus ring: its rows, the active one, and whether the keyboard has the list. */
const ringRows = ['Rename', 'Duplicate', 'Archive']
const ringActive = ref(0)
const ringKeyboard = ref(false)

function ringListFocus(event: FocusEvent): void {
  ringKeyboard.value = focusedByKeyboard(event)
}

function ringListKey(event: KeyboardEvent): void {
  const step = event.key === 'ArrowDown' ? 1 : event.key === 'ArrowUp' ? -1 : 0
  if (step === 0)
    return
  event.preventDefault()
  ringKeyboard.value = true
  ringActive.value = (ringActive.value + step + ringRows.length) % ringRows.length
}

/** A clipboard that refuses, which the failure specimen of CopyButton copies to. */
function refusedCopy(): never {
  throw new Error('This page may not write to the clipboard.')
}

const dotTones: readonly StatusDotTone[] = ['accent', 'success', 'warning', 'danger', 'attention', 'muted']
const dotBreathing = ref(true)

const query = ref('session cookie')
const emptyQuery = ref('')
const secret = ref('sk-ant-api03-example-not-a-real-key')
const accountPassword = ref('correct horse battery')
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
const cardLong = useCardLimits('alex@example.com')
/** Each cost specimen's conversation, read as its card opens, as the product reads it. */
const costClosed = useCardCost('profiles')
const costProfiles = useCardCost('profiles')
const costPlain = useCardCost('plain')
const costAlone = useCardCost('none')
const costFailed = useCardCost('profiles', { failing: true })
const costYoung = useCardCost('none', { turns: 2 })
const costShort = useCardCost('profiles', { turns: 12 })
const costGiant = useCardCost('profiles', { turns: 1_000 })
const costMany = useCardCost('many')
const costSubagent = useCardCost('none', { turns: 60 })
const costUnpriced = useCardCost('none', { turns: 30, priced: false })
const costEarly = useCardCost('profiles')
const costWindowless = useCardCost('none', { turns: 12 })
const costCompacting = useCardCost('profiles')
// Each call by its place among the conversation's 180; the chart draws the last 50, from #131.
const costTips = [
  { variant: 'tip · a call', call: 20, cost: useCardCost('profiles', { turns: 40 }) },
  { variant: 'tip · a miss, its cause in red', call: 6, cost: useCardCost('profiles', { turns: 40 }) },
  { variant: 'tip · the first call after a compaction', call: 30, cost: useCardCost('profiles', { turns: 40 }) },
  { variant: 'tip · near the card’s end, above its point', call: 38, cost: useCardCost('profiles', { turns: 40 }) },
  { variant: 'tip · a point of 10 calls: their numbers, rate and price', call: 849, cost: useCardCost('profiles', { turns: 1_000 }) },
  { variant: 'tip · a point of 10 calls with a miss, its cause in red', call: 678, cost: useCardCost('profiles', { turns: 1_000 }) },
]
const cardTruncated = useCardLimits('a-very-long-account-name.with.many.parts@subdomain.example.com')
const cardUnlabeled = useCardLimits('')
const cardNew = useCardLimits('alex@example.com')
const cardEarly = useCardLimits('alex@example.com')
const cardWindowless = useCardLimits('alex@example.com')
const cardFailing = useCardLimits('alex@example.com', 'Anthropic’s usage service is unavailable.')
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

/** Every logo a device's mark takes. */
const DEVICE_LOGOS: DeviceLogo[] = ['apple', 'windows', 'ubuntu', 'debian', 'fedora', 'arch', 'centos', 'redhat', 'mint', 'tux']
/** Marks with a number, as two or more devices of a logo show it, a two-digit one included. */
const DEVICE_NUMBERS: { variant: string; mark: DeviceMark }[] = [
  { variant: 'Apple 1', mark: { logo: 'apple', number: 1 } },
  { variant: 'Apple 2', mark: { logo: 'apple', number: 2 } },
  { variant: 'Windows 3', mark: { logo: 'windows', number: 3 } },
  { variant: 'Ubuntu 4', mark: { logo: 'ubuntu', number: 4 } },
  { variant: 'Tux 7', mark: { logo: 'tux', number: 7 } },
  { variant: 'Apple 12', mark: { logo: 'apple', number: 12 } },
  { variant: 'Debian 10', mark: { logo: 'debian', number: 10 } },
]
/** Each state, with and without a number, and a device since removed. */
const DEVICE_STATES: { variant: string; state: DeviceState | null; mark: DeviceMark | null }[] = [
  { variant: 'online', state: 'online', mark: { logo: 'apple', number: 2 } },
  { variant: 'updating', state: 'updating', mark: { logo: 'apple', number: 2 } },
  { variant: 'offline', state: 'offline', mark: { logo: 'apple', number: 2 } },
  { variant: 'online, alone', state: 'online', mark: { logo: 'windows', number: null } },
  { variant: 'updating, alone', state: 'updating', mark: { logo: 'windows', number: null } },
  { variant: 'offline, alone', state: 'offline', mark: { logo: 'windows', number: null } },
  { variant: 'removed', state: null, mark: null },
]
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
            <Button disabled :disabled-reason="inDevelopment()">In Development</Button>
          </GallerySpecimen>
        </div>
      </GallerySection>

      <GallerySection
        title="Working"
        note="No click goes unanswered. A control is bound to an operation, an action on one target, and its handler returns the operation’s promise: the page keeps the running operations, not the control, so a menu or a dialog opened again shows the operation working and a press meanwhile starts nothing. A button keeps its look, a danger button its red, and shows the spinner in place of its icon or beside its label; an icon button shows it in place of its icon; a menu item closes its menu and the row of its target shows the spinner, and the item opened again shows it too (a choice, which keeps the menu open, shows it in its check gutter); a row action shows it on its row. Operations that exclude each other on one target, such as a dialog’s Save and Delete, form a group: while one works the other cannot be pressed, and Cancel always can. Closing a dialog while it works leaves the progress on its row. Each specimen waits 0.9 s, the menu and the dialog 3 s."
      >
        <GalleryWorkingControls />
      </GallerySection>

      <GallerySection
        title="Focus Ring"
        note="What the keyboard’s focus shows on any control, and only that: a click leaves none. A 2 px ring 1 px off its control, so it reaches 3 px past it; press Tab to see it. A box that clips (overflow other than visible, contain: paint) cuts what reaches past its edge, so inside one the ring is drawn inside its control: every overflow utility sets that for what it holds, and a box that clips by its own CSS carries focus-ring-inset. Where the page moves the focus (a closed window giving it back, a field focused on opening), none shows, and a link in running text takes its hover look instead of a ring. A list that holds the focus and names its active row (aria-activedescendant) draws the ring on that row with focus-ring-shown; a selection is shown by its own selected look, never by the ring. bun browse finds a ring a box cuts or an element covers (demi.focusRings)."
      >
        <div class="specimen-row">
          <GallerySpecimen variant="in the open">
            <div class="flex items-center gap-2 p-1">
              <Button @click="productWould('Saved')">Save</Button>
              <IconButton :icon="Search" aria-label="Search" @click="productWould('Search Opened')" />
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="in a box that clips · overflow-hidden">
            <div class="w-48 overflow-hidden rounded-lg border border-line">
              <button
                v-for="row in ['Rename', 'Duplicate', 'Archive']"
                :key="row"
                type="button"
                class="flex h-8 w-full cursor-default items-center px-3 text-left text-chrome hover:bg-hover"
                @click="productWould(`${row} Chosen`)"
              >{{ row }}</button>
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="in a box that clips by its own CSS · focus-ring-inset">
            <div class="focus-ring-inset w-48 rounded-lg border border-line" style="overflow: clip">
              <button
                v-for="row in ['Rename', 'Duplicate', 'Archive']"
                :key="row"
                type="button"
                class="flex h-8 w-full cursor-default items-center px-3 text-left text-chrome hover:bg-hover"
                @click="productWould(`${row} Chosen`)"
              >{{ row }}</button>
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="a link in running text · hover look, no ring">
            <p class="markdown-body w-72">
              Read <a
                href="#"
                @click.prevent="productWould('Link Opened')"
              >the guide</a> first, or the <ExternalLink
                href="https://example.com"
                @click.prevent="productWould('Site Opened')"
              >site</ExternalLink>.
            </p>
          </GallerySpecimen>
          <GallerySpecimen variant="a list's active row · focus-ring-shown">
            <div
              role="listbox"
              tabindex="0"
              aria-label="Actions"
              :aria-activedescendant="`ring-row-${ringActive}`"
              class="focus-ring-inset w-48 rounded-lg border border-line outline-none"
              @focus="ringListFocus"
              @blur="ringKeyboard = false"
              @keydown="ringListKey"
            >
              <div
                v-for="(row, index) in ringRows"
                :id="`ring-row-${index}`"
                :key="row"
                role="option"
                :aria-selected="index === ringActive"
                class="flex h-8 w-full cursor-default items-center px-3 text-chrome hover:bg-hover"
                :class="[index === ringActive ? 'bg-active' : '', index === ringActive && ringKeyboard ? 'focus-ring-shown' : '']"
                @click="ringActive = index; productWould(`${row} Chosen`)"
              >{{ row }}</div>
            </div>
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
              :disabled-reason="inDevelopment()"
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
              :disabled-reason="inDevelopment()"
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
              :disabled-reason="inDevelopment()"
              label="Show unreads"
            />
          </GallerySpecimen>
        </div>
      </GallerySection>

      <GallerySection
        title="TextInput"
        note="Empty, filled, focus, bare (no frame in any state), a search box, an account password and a secret, each with its eye, a suffix, and the 36px lg family for page forms. Only an account password (sign-in, setup, change password) is a real password field, with its autocomplete token, so the browser offers and saves the user’s login there. Every other secret (an API key, a token, a credential’s value) is masked secret: a text field drawn as dots that browsers and password managers neither fill nor save. A filter or search box is search: it is never taken for a login’s user name."
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
          <GallerySpecimen variant="search">
            <TextInput v-model="query" placeholder="Filter settings" search>
              <template #prefix><Search :size="14" /></template>
            </TextInput>
          </GallerySpecimen>
          <GallerySpecimen variant="account password">
            <TextInput
              v-model="accountPassword"
              placeholder="Password"
              masked="password"
              autocomplete="current-password"
            />
          </GallerySpecimen>
          <GallerySpecimen variant="secret">
            <TextInput
              v-model="secret"
              placeholder="sk-…"
              masked="secret"
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
              masked="secret"
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
            <SwatchPicker v-model="swatch" :options="productAccentOptions()" />
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
        title="Avatar"
        note="An account’s picture until pictures arrive: its initials in white on a muted tone, a gentle gradient from top to bottom. The account’s id picks the tone from eight, so one person has the same tone in the sidebar, Settings and the Users list. The letters are the first of the first two words of the name, one for a single word or a single Chinese character, else the first of the email’s local part. 24px is the sidebar’s and the settings rail’s, 32px the phone sidebar’s, Settings › Account’s and the Users list’s. Switch the appearance to see the other scheme."
      >
        <div class="grid grid-cols-1 gap-6 lg:grid-cols-2">
          <div class="min-w-0 space-y-4 rounded-xl bg-surface p-4">
            <div class="space-y-1">
              <div class="gallery-label">Sidebar · 24px</div>
              <div class="flex h-9 items-center gap-2 rounded-md bg-surface-base pl-2 pr-1.5">
                <Avatar :account-id="avatarSelf.id" :name="avatarSelf.name" :email="avatarSelf.email" :size="24" />
                <span class="min-w-0 truncate text-chrome text-fg">{{ accountDisplayName(avatarSelf.name, avatarSelf.email) }}</span>
              </div>
              <div class="gallery-label pt-2">Sidebar on a phone · 32px</div>
              <div class="flex h-11 items-center gap-3 rounded-md bg-surface-base pl-2 pr-1.5">
                <Avatar :account-id="avatarSelf.id" :name="avatarSelf.name" :email="avatarSelf.email" :size="32" />
                <span class="min-w-0 truncate text-[16px] text-fg">{{ accountDisplayName(avatarSelf.name, avatarSelf.email) }}</span>
              </div>
            </div>
            <div class="space-y-1">
              <div class="gallery-label">The eight tones · slate, sage, clay, sand, dusk, moss, plum, steel</div>
              <div class="flex gap-2">
                <Avatar v-for="tone in avatarPalette" :key="tone.id" :account-id="tone.id" :name="tone.name" :size="32" />
              </div>
            </div>
          </div>
          <div class="min-w-0 space-y-4 rounded-xl bg-surface p-4">
            <div class="space-y-1">
              <div class="gallery-label">Settings › Account · 32px</div>
              <SettingsGroup>
                <SettingsRow label="Avatar" description="Your initials.">
                  <Avatar :account-id="avatarSelf.id" :name="avatarSelf.name" :email="avatarSelf.email" :size="32" />
                </SettingsRow>
              </SettingsGroup>
            </div>
            <div class="space-y-1">
              <div class="gallery-label">Settings › Users · 32px</div>
              <SettingsGroup>
                <SettingsRow v-for="account in avatarAccounts" :key="account.id" :label="accountDisplayName(account.name, account.email)" :description="account.name ? account.email : undefined" center-leading>
                  <template #leading>
                    <Avatar :account-id="account.id" :name="account.name" :email="account.email" :size="32" />
                  </template>
                </SettingsRow>
              </SettingsGroup>
            </div>
          </div>
        </div>
      </GallerySection>

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
        note="A device’s mark, the same wherever a device is drawn: its system’s logo in the subtle icon color, or at a sidebar row’s end in the grey of the row’s buttons, its state’s dot at the logo’s top right, green online, amber updating, faint offline, and under the dot the number that tells it from the user’s other devices of the same logo, shown only while another has it. The dot and the number are cut out of what is behind them. A two-digit number reaches further left; a device since removed is the plain computer, without a dot."
      >
        <!-- One grid, so a row that wraps and the next set sit the same distance apart. -->
        <div class="specimen-row text-fg-subtle">
          <GallerySpecimen v-for="logo in DEVICE_LOGOS" :key="logo" :variant="logo">
            <DeviceIcon state="online" :mark="{ logo, number: null }" />
          </GallerySpecimen>
          <GallerySpecimen
            v-for="specimen in DEVICE_NUMBERS"
            :key="specimen.variant"
            :variant="specimen.variant"
          >
            <DeviceIcon state="online" :mark="specimen.mark" />
          </GallerySpecimen>
          <GallerySpecimen
            v-for="specimen in DEVICE_STATES"
            :key="specimen.variant"
            :variant="specimen.variant"
          >
            <DeviceIcon :state="specimen.state" :mark="specimen.mark" />
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
        note="The ring is the backend’s estimate, blue below 70% of the window, orange from 70% and red from 90%. Click it for the card, on a phone tap it for a sheet; a click outside, another click on the ring or Escape closes it, and resting the pointer on it shows only its tip, the word Context and the figure the card’s header shows, such as Context 139K / 1M (13%), and none without a usage or a window. Compact closes the card, as a menu’s action does. The header opens and closes the list of what fills the context, while it compacts too, where it says Compacting; one part of the card stands open at a time, the list or the cost’s details, closed at first and then as the user left it, kept in the browser; the bar shows either way, the rows largest first, each one figure with its share of the window. Under them, the room before compaction, with Compact, from half the window, or at any share after a turn the vendor refused for its size. Under it, once the conversation made a call, what it cost, which the page reads with the conversation as it opens and follows while it is open, so the card shows it the moment it opens: closed, the equivalent API price and the main conversation’s cache hit rate over the calls its chart draws side by side, each its value over its label; its caption is a disclosure of its own, and opening it closes the list. Opened, the caption holds the price, then the price by kind of token two by two, the Profiles part in two columns (the main conversation, then the five profiles the conversation’s workflow offers whose subagents cost the most, a bar under each of its share of the largest), and from the fourth call the chart of the turn calls’ hit rates by their numbers in the conversation: at most 40 points over the last 400 calls, consecutive calls merged into one point as the conversation grows, the groups counted from call 1 so a point never moves, round numbers under it, a point with a miss a red dot, a compaction a grey dashed line. Resting the pointer on the chart, or a tap on a phone, shows the nearest point’s tip beside it where it fits, otherwise above it, never past the card’s edges: a call’s number, time, rate and price, a miss’s cause in red with the tokens it wrote again, and First request or After compaction in grey; a group’s range of numbers, rate, summed price and its misses’ causes. A figure that cannot be computed is left out with its label. Then the usage limits of a subscription account, under a line that ends in a Refresh usage button (its icon turns while the request runs, the windows stay until the answer replaces them, a failure shows the vendor’s words under the line until the next answer, and a click keeps the card open), each window’s name, reset and use on one line over its bar (a model on an API key has none), and last the instruction files, each of which opens where it is written. While the usage or the window is unknown, as in a conversation just reopened before its session’s estimate or on a model without a window, the card holds the cost and the usage limits, and a model with no limits has no meter, whatever it cost."
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
              unavailable-reason="Available after the turn ends"
            />
          </GallerySpecimen>
          <GallerySpecimen variant="compacting">
            <ContextUsageIndicator
              :usage="usageAt(0.75)"
              :is-compacting="true"
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
            <GallerySpecimen variant="card · turn refused for its size, Compact at 28%">
              <div class="px-44 pt-[36rem]">
                <ContextUsageIndicator
                  pinned
                  size-refused
                  :usage="usageAt(0.28)"
                  :instructions="demoInstructions"
                  @compact="productWould('Compact the Conversation')"
                  @open-instruction="demoOpenInstruction($event, productWould)"
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
            <GallerySpecimen variant="card · new conversation, the usage limits alone">
              <div class="px-44 pt-[16rem]">
                <ContextUsageIndicator
                  pinned
                  :usage="null"
                  :instructions="demoInstructions"
                  :limits="cardNew.limits.value"
                  @refresh-limits="cardNew.refresh"
                />
              </div>
            </GallerySpecimen>
          </GalleryOverlayWell>
          <GalleryOverlayWell>
            <GallerySpecimen variant="card · Cost closed, its price and cache hit rate">
              <div class="px-44 pt-[36rem]">
                <ContextUsageIndicator
                  pinned
                  :expanded="null"
                  :usage="usageAt(0.62)"
                  :instructions="demoInstructions"
                  :limits="cardLong.limits.value"
                  :cost="costClosed.cost.value"
                  :cost-profiles="OFFERED_PROFILES"
                  @compact="productWould('Compact the Conversation')"
                  @open-instruction="demoOpenInstruction($event, productWould)"
                  @refresh-limits="cardLong.refresh"
                  @retry-cost="costClosed.retry"
                />
              </div>
            </GallerySpecimen>
          </GalleryOverlayWell>
          <GalleryOverlayWell size="tall">
            <GallerySpecimen variant="card · Cost opened, 180 calls of 5 a point, the profiles Global Workflow offers; a disabled one and none have no row">
              <div class="px-44 pt-[70rem]">
                <ContextUsageIndicator
                  pinned
                  expanded="cost"
                  :usage="usageAt(0.62)"
                  :instructions="demoInstructions"
                  :cost="costProfiles.cost.value"
                  :cost-profiles="OFFERED_PROFILES"
                  @compact="productWould('Compact the Conversation')"
                  @open-instruction="demoOpenInstruction($event, productWould)"
                  @retry-cost="costProfiles.retry"
                />
              </div>
            </GallerySpecimen>
          </GalleryOverlayWell>
          <GalleryOverlayWell size="tall">
            <GallerySpecimen variant="card · Cost opened, subagents without profiles: no Profiles part">
              <div class="px-44 pt-[70rem]">
                <ContextUsageIndicator
                  pinned
                  expanded="cost"
                  :usage="usageAt(0.62)"
                  :instructions="demoInstructions"
                  :cost="costPlain.cost.value"
                  :cost-profiles="OFFERED_PROFILES"
                  @compact="productWould('Compact the Conversation')"
                  @open-instruction="demoOpenInstruction($event, productWould)"
                  @retry-cost="costPlain.retry"
                />
              </div>
            </GallerySpecimen>
          </GalleryOverlayWell>
          <GalleryOverlayWell size="tall">
            <GallerySpecimen variant="card · Cost opened, no subagents">
              <div class="px-44 pt-[62rem]">
                <ContextUsageIndicator
                  pinned
                  expanded="cost"
                  :usage="usageAt(0.62)"
                  :instructions="demoInstructions"
                  :cost="costAlone.cost.value"
                  :cost-profiles="OFFERED_PROFILES"
                  @compact="productWould('Compact the Conversation')"
                  @open-instruction="demoOpenInstruction($event, productWould)"
                  @retry-cost="costAlone.retry"
                />
              </div>
            </GallerySpecimen>
          </GalleryOverlayWell>
          <GalleryOverlayWell size="tall">
            <GallerySpecimen variant="card · Cost opened, two calls: the rate without the chart">
              <div class="px-44 pt-[40rem]">
                <ContextUsageIndicator
                  pinned
                  expanded="cost"
                  :usage="usageAt(0.12)"
                  :instructions="demoInstructions"
                  :cost="costYoung.cost.value"
                  :cost-profiles="OFFERED_PROFILES"
                  @compact="productWould('Compact the Conversation')"
                  @open-instruction="demoOpenInstruction($event, productWould)"
                  @retry-cost="costYoung.retry"
                />
              </div>
            </GallerySpecimen>
          </GalleryOverlayWell>
          <GalleryOverlayWell>
            <GallerySpecimen variant="card · Cost, a read that failed, with Retry">
              <div class="px-44 pt-[30rem]">
                <ContextUsageIndicator
                  pinned
                  :expanded="null"
                  :usage="usageAt(0.62)"
                  :instructions="demoInstructions"
                  :cost="costFailed.cost.value"
                  :cost-profiles="OFFERED_PROFILES"
                  @compact="productWould('Compact the Conversation')"
                  @open-instruction="demoOpenInstruction($event, productWould)"
                  @retry-cost="costFailed.retry"
                />
              </div>
            </GallerySpecimen>
          </GalleryOverlayWell>
          <GalleryOverlayWell size="tall">
            <GallerySpecimen variant="card · Cost opened, 12 calls: one call a point">
              <div class="px-44 pt-[50rem]">
                <ContextUsageIndicator
                  pinned
                  expanded="cost"
                  :usage="usageAt(0.22)"
                  :instructions="demoInstructions"
                  :cost="costShort.cost.value"
                  :cost-profiles="OFFERED_PROFILES"
                  @compact="productWould('Compact the Conversation')"
                  @open-instruction="demoOpenInstruction($event, productWould)"
                  @retry-cost="costShort.retry"
                />
              </div>
            </GallerySpecimen>
          </GalleryOverlayWell>
          <GalleryOverlayWell size="tall">
            <GallerySpecimen variant="card · Cost opened, 1,000 calls: the last 400, 10 a point">
              <div class="px-44 pt-[50rem]">
                <ContextUsageIndicator
                  pinned
                  expanded="cost"
                  :usage="usageAt(0.62)"
                  :instructions="demoInstructions"
                  :cost="costGiant.cost.value"
                  :cost-profiles="OFFERED_PROFILES"
                  @compact="productWould('Compact the Conversation')"
                  @open-instruction="demoOpenInstruction($event, productWould)"
                  @retry-cost="costGiant.retry"
                />
              </div>
            </GallerySpecimen>
          </GalleryOverlayWell>
          <GalleryOverlayWell size="tall">
            <GallerySpecimen variant="card · Cost opened, eight profiles: the main conversation and the five that cost the most">
              <div class="px-44 pt-[50rem]">
                <ContextUsageIndicator
                  pinned
                  expanded="cost"
                  :usage="usageAt(0.62)"
                  :instructions="demoInstructions"
                  :cost="costMany.cost.value"
                  :cost-profiles="MANY_PROFILES"
                  @compact="productWould('Compact the Conversation')"
                  @open-instruction="demoOpenInstruction($event, productWould)"
                  @retry-cost="costMany.retry"
                />
              </div>
            </GallerySpecimen>
          </GalleryOverlayWell>
          <GalleryOverlayWell size="tall">
            <GallerySpecimen variant="card · Cost opened, a model with no prices: no price by kind, the rate and the chart">
              <div class="px-44 pt-[50rem]">
                <ContextUsageIndicator
                  pinned
                  expanded="cost"
                  :usage="usageAt(0.3)"
                  :cost="costUnpriced.cost.value"
                  :cost-profiles="OFFERED_PROFILES"
                  @compact="productWould('Compact the Conversation')"
                  @retry-cost="costUnpriced.retry"
                />
              </div>
            </GallerySpecimen>
          </GalleryOverlayWell>
          <GalleryOverlayWell size="tall">
            <GallerySpecimen variant="card · a subagent’s Cost: no Profiles part">
              <div class="px-44 pt-[50rem]">
                <ContextUsageIndicator
                  pinned
                  expanded="cost"
                  :usage="usageAt(0.34)"
                  :cost="costSubagent.cost.value"
                  :cost-profiles="[]"
                  @compact="productWould('Compact the Conversation')"
                  @retry-cost="costSubagent.retry"
                />
              </div>
            </GallerySpecimen>
          </GalleryOverlayWell>
          <GalleryOverlayWell>
            <GallerySpecimen variant="card · reopened, before the session’s estimate: the cost and the usage limits">
              <div class="px-44 pt-[30rem]">
                <ContextUsageIndicator
                  pinned
                  :expanded="null"
                  :usage="null"
                  :instructions="demoInstructions"
                  :limits="cardEarly.limits.value"
                  :cost="costEarly.cost.value"
                  :cost-profiles="OFFERED_PROFILES"
                  @refresh-limits="cardEarly.refresh"
                  @retry-cost="costEarly.retry"
                />
              </div>
            </GallerySpecimen>
          </GalleryOverlayWell>
          <GalleryOverlayWell>
            <GallerySpecimen variant="card · a model without a window, on a subscription: the cost and the usage limits">
              <div class="px-44 pt-[30rem]">
                <ContextUsageIndicator
                  pinned
                  :expanded="null"
                  :usage="usageOf(42_000, null)"
                  :limits="cardWindowless.limits.value"
                  :cost="costWindowless.cost.value"
                  :cost-profiles="OFFERED_PROFILES"
                  @refresh-limits="cardWindowless.refresh"
                  @retry-cost="costWindowless.retry"
                />
              </div>
            </GallerySpecimen>
          </GalleryOverlayWell>
          <GalleryOverlayWell>
            <GallerySpecimen variant="card · compacting: the list and the cost still open">
              <div class="px-44 pt-[44rem]">
                <ContextUsageIndicator
                  pinned
                  is-compacting
                  expanded="context"
                  :usage="usageAt(0.62)"
                  :instructions="demoInstructions"
                  :limits="cardLong.limits.value"
                  :cost="costCompacting.cost.value"
                  :cost-profiles="OFFERED_PROFILES"
                  @open-instruction="demoOpenInstruction($event, productWould)"
                  @refresh-limits="cardLong.refresh"
                  @retry-cost="costCompacting.retry"
                />
              </div>
            </GallerySpecimen>
          </GalleryOverlayWell>
          <GalleryOverlayWell v-for="tip in costTips" :key="tip.variant" size="tall">
            <GallerySpecimen :variant="tip.variant">
              <div class="px-44 pt-[62rem]">
                <ContextUsageIndicator
                  pinned
                  expanded="cost"
                  :usage="usageAt(0.62)"
                  :cost="tip.cost.cost.value"
                  :cost-profiles="OFFERED_PROFILES"
                  :cost-tip="tip.call"
                  @compact="productWould('Compact the Conversation')"
                  @retry-cost="tip.cost.retry"
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
        note="A vendor’s mark in the text’s full ink, hidden from assistive technology since a name always stands beside it, found by the vendor’s ID among the marks’ files (vendor-logos), models.dev’s or kept by hand: a subscription wears its vendor’s, Claude Code Claude’s own; a vendor without a mark shows its initial. Bare at a row’s size as a menu row’s icon or a dropdown’s face, small on a tile in lists, medium beside a title."
      >
        <div class="specimen-row">
          <GallerySpecimen v-for="mark in vendorMarks" :key="mark.label" :variant="mark.label">
            <div class="flex items-center gap-2">
              <VendorMark :label="mark.label" :src="mark.src" size="row" />
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
