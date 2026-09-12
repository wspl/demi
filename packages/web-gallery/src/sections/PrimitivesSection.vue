<script setup lang="ts">
import { ref } from 'vue'
import { ChevronDown, RefreshCw, Plus, Search, Send, Settings2, Trash2 } from '@lucide/vue'
import CopyCode from '@demicodes/web-ui/ui/CopyCode.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Checkbox from '@demicodes/web-ui/ui/Checkbox.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import IndeterminateSpinner from '@demicodes/web-ui/ui/IndeterminateSpinner.vue'
import Switch from '@demicodes/web-ui/ui/Switch.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import ScrollArea from '@demicodes/web-ui/ui/ScrollArea.vue'
import ShortcutRecorder from '@demicodes/web-ui/ui/ShortcutRecorder.vue'
import SwatchPicker from '@demicodes/web-ui/ui/SwatchPicker.vue'
import ResizeHandle from '@demicodes/web-ui/ui/ResizeHandle.vue'
import ChoiceCards from '@demicodes/web-ui/ui/ChoiceCards.vue'
import { Cloud, Monitor } from '@lucide/vue'
import { PRODUCT_ACCENTS } from '@demicodes/web-ui/theme/productAppearance'
import TokenInput from '@demicodes/web-ui/ui/TokenInput.vue'
import HighlightText from '@demicodes/web-ui/ui/HighlightText.vue'
import ThemeToggle from '@demicodes/web-ui/ui/ThemeToggle.vue'
import ConversationStatusDot from '@demicodes/web-ui/agent/ConversationStatusDot.vue'
import ContextUsageIndicator from '@demicodes/web-ui/agent/ContextUsageIndicator.vue'
import ProviderIcon from '@demicodes/web-ui/agent/providers/ProviderIcon.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import { IN_DEVELOPMENT } from '@demicodes/web-ui/ui/disabled'
import { demoUsage } from '../fixtures/blocks'
import { usageAt } from '../fixtures/catalog'
import GallerySection from '../components/GallerySection.vue'
import GallerySpecimen from '../components/GallerySpecimen.vue'
import { useGalleryView } from '../gallery-views'

const { view } = useGalleryView()
const paneWidth = ref(200)
const paneHeight = ref(96)

const spinning = ref(false)

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
] as const
const checkboxOff = ref(false)
const checkboxPartialOn = ref(false)
const checkboxPartial = ref(true)
const compactIdle = ref(false)
const compactWarn = ref(false)
const compactDanger = ref(false)
const buttonPressed = ref(true)
const buttonGhostPressed = ref(true)
const iconPressed = ref(true)
const iconCirclePressed = ref(true)

function pulseCompact(which: 'idle' | 'warn' | 'danger'): void {
  const flag = which === 'idle'
    ? compactIdle
    : which === 'warn'
    ? compactWarn
    : compactDanger
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
        note="Enabled, disabled, sizes, and pressed."
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
              />{{ spinning ? 'Stop spinning' : 'Start spinning' }}</Button>
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
            <Button size="md" variant="primary">Primary</Button>
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
            <Button size="xs">Extra small</Button>
          </GallerySpecimen>
          <GallerySpecimen variant="primary · xs">
            <Button size="xs" variant="primary">Extra small</Button>
          </GallerySpecimen>
          <GallerySpecimen variant="ghost · xs">
            <Button size="xs" variant="ghost">Extra small</Button>
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
          <GallerySpecimen variant="disabled · reason">
            <Button disabled :disabled-reason="IN_DEVELOPMENT">In development</Button>
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
        <div class="specimen-row">
          <GallerySpecimen variant="accents">
            <SwatchPicker v-model="swatch" :options="PRODUCT_ACCENTS" />
          </GallerySpecimen>
        </div>
      </GallerySection>

      <GallerySection
        title="ChoiceCards"
        note="A few exclusive choices as cards with an icon, a title and a line, for a form that branches on the answer."
      >
        <div class="specimen-row">
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
        note="The bar takes no room: a thumb drawn over the content, shown on hover or while scrolling, draggable."
      >
        <div class="specimen-row">
          <GallerySpecimen variant="overflowing">
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
        <div class="specimen-row">
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
        note="Idle, warning, danger, compacting, and unavailable."
      >
        <div class="specimen-row">
          <GallerySpecimen variant="idle · 34%">
            <ContextUsageIndicator
              conversation-id="demo"
              :usage="demoUsage"
              :context-window="200000"
              :input-limit="180000"
              :is-compacting="compactIdle"
              :is-clickable="true"
              @compact="pulseCompact('idle')"
            />
          </GallerySpecimen>
          <GallerySpecimen variant="warning · 75%">
            <ContextUsageIndicator
              conversation-id="demo-warn"
              :usage="usageAt(0.75)"
              :context-window="200000"
              :input-limit="180000"
              :is-compacting="compactWarn"
              :is-clickable="true"
              @compact="pulseCompact('warn')"
            />
          </GallerySpecimen>
          <GallerySpecimen variant="danger · 94%">
            <ContextUsageIndicator
              conversation-id="demo-danger"
              :usage="usageAt(0.94)"
              :context-window="200000"
              :input-limit="180000"
              :is-compacting="compactDanger"
              :is-clickable="true"
              @compact="pulseCompact('danger')"
            />
          </GallerySpecimen>
          <GallerySpecimen variant="compacting">
            <ContextUsageIndicator
              conversation-id="demo-compact"
              :usage="demoUsage"
              :context-window="200000"
              :input-limit="180000"
              :is-compacting="true"
            />
          </GallerySpecimen>
          <GallerySpecimen variant="unavailable">
            <ContextUsageIndicator conversation-id="demo-none" />
          </GallerySpecimen>
        </div>
      </GallerySection>

      <GallerySection title="ProviderIcon" note="Anthropic and OpenAI.">
        <div class="specimen-row">
          <GallerySpecimen variant="anthropic">
            <ProviderIcon provider-id="anthropic" :size="ICON_PX.markIn28" />
          </GallerySpecimen>
          <GallerySpecimen variant="openai">
            <ProviderIcon provider-id="openai" :size="ICON_PX.markIn28" />
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
