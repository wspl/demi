<script setup lang="ts">
import { ref } from 'vue'
import { ChevronDown, Plus, Search, Send, Settings2, Trash2 } from '@lucide/vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Checkbox from '@demicodes/web-ui/ui/Checkbox.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import IndeterminateSpinner from '@demicodes/web-ui/ui/IndeterminateSpinner.vue'
import Switch from '@demicodes/web-ui/ui/Switch.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import HighlightText from '@demicodes/web-ui/ui/HighlightText.vue'
import ThemeToggle from '@demicodes/web-ui/ui/ThemeToggle.vue'
import ConversationStatusDot from '@demicodes/web-ui/agent/ConversationStatusDot.vue'
import ContextUsageIndicator from '@demicodes/web-ui/agent/ContextUsageIndicator.vue'
import ProviderIcon from '@demicodes/web-ui/agent/providers/ProviderIcon.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import { demoUsage } from '../fixtures/blocks'
import { usageAt } from '../fixtures/catalog'
import GallerySection from '../components/GallerySection.vue'
import GallerySpecimen from '../components/GallerySpecimen.vue'

const query = ref('session cookie')
const emptyQuery = ref('')
const enabled = ref(true)
const enabledOff = ref(false)
const enabledSmOn = ref(true)
const enabledSmOff = ref(false)
const checkboxOn = ref(true)
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
  const flag = which === 'idle' ? compactIdle : which === 'warn' ? compactWarn : compactDanger
  flag.value = true
  window.setTimeout(() => {
    flag.value = false
  }, 1200)
}
</script>

<template>
  <div class="space-y-8">
    <GallerySection title="Button" note="Enabled, disabled, sizes, and pressed.">
      <div class="specimen-row">
        <GallerySpecimen variant="default">
          <Button size="md">Default</Button>
        </GallerySpecimen>
        <GallerySpecimen variant="default · pressed">
          <Button size="md" :pressed="buttonPressed" @click="buttonPressed = !buttonPressed">Default</Button>
        </GallerySpecimen>
        <GallerySpecimen variant="primary">
          <Button size="md" variant="primary">Primary</Button>
        </GallerySpecimen>
        <GallerySpecimen variant="ghost">
          <Button size="md" variant="ghost">Ghost</Button>
        </GallerySpecimen>
        <GallerySpecimen variant="ghost · pressed">
          <Button size="md" variant="ghost" :pressed="buttonGhostPressed" @click="buttonGhostPressed = !buttonGhostPressed">Ghost</Button>
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
      </div>
    </GallerySection>

    <GallerySection title="IconButton" note="Chip and circle. Accent is send.">
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
          <IconButton :icon="Plus" variant="ghost" :pressed="iconPressed" @click="iconPressed = !iconPressed" />
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
            <IconButton :icon="Plus" variant="ghost" circle />
          </div>
        </GallerySpecimen>
        <GallerySpecimen variant="circle · pressed">
          <IconButton :icon="Plus" variant="ghost" circle :pressed="iconCirclePressed" @click="iconCirclePressed = !iconCirclePressed" />
        </GallerySpecimen>
        <GallerySpecimen variant="circle · accent">
          <IconButton :icon="Send" variant="accent" circle />
        </GallerySpecimen>
        <GallerySpecimen variant="circle · danger">
          <IconButton :icon="Trash2" variant="danger" circle />
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
            <IconButton :icon="Send" variant="accent" disabled />
            <IconButton :icon="Trash2" variant="danger" disabled />
          </div>
        </GallerySpecimen>
      </div>
    </GallerySection>

    <GallerySection title="Switch" note="On and off, md and sm.">
      <div class="specimen-row">
        <GallerySpecimen variant="md · on">
          <Switch v-model="enabled" label="Available" />
        </GallerySpecimen>
        <GallerySpecimen variant="md · off">
          <Switch v-model="enabledOff" label="Available" />
        </GallerySpecimen>
        <GallerySpecimen variant="sm · on">
          <Switch v-model="enabledSmOn" size="sm" label="Compact" />
        </GallerySpecimen>
        <GallerySpecimen variant="sm · off">
          <Switch v-model="enabledSmOff" size="sm" label="Compact" />
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
          <Checkbox v-model="checkboxPartialOn" v-model:partial="checkboxPartial" label="Show unreads" />
        </GallerySpecimen>
      </div>
    </GallerySection>

    <GallerySection title="TextInput" note="Empty, filled, focus, and suffix.">
      <div class="specimen-row">
        <GallerySpecimen variant="empty">
          <TextInput v-model="emptyQuery" placeholder="Search files" />
        </GallerySpecimen>
        <GallerySpecimen variant="filled">
          <TextInput v-model="query" placeholder="Search files" />
        </GallerySpecimen>
        <GallerySpecimen variant="focused">
          <TextInput v-model="query" placeholder="Search files" focused />
        </GallerySpecimen>
        <GallerySpecimen variant="suffix">
          <TextInput v-model="query" placeholder="With suffix">
            <template #suffix>
              <Search :size="14" class="text-fg-subtle" />
            </template>
          </TextInput>
        </GallerySpecimen>
      </div>
    </GallerySection>

    <GallerySection title="ConversationStatusDot" note="Idle, active, done, error, and aborted.">
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

    <GallerySection title="IndeterminateSpinner" note="14px in-progress mark.">
      <div class="specimen-row">
        <GallerySpecimen variant="14">
          <IndeterminateSpinner />
        </GallerySpecimen>
      </div>
    </GallerySection>

    <GallerySection title="ContextUsageIndicator" note="Idle, warning, danger, compacting, and unavailable.">
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

    <GallerySection title="HighlightText" note="Match, multi-match, none, and empty query.">
      <div class="specimen-row">
        <GallerySpecimen variant="match">
          <div class="text-[13px] text-fg-body">
            <HighlightText text="session cookie after the rename" query="session" />
          </div>
        </GallerySpecimen>
        <GallerySpecimen variant="multi-match">
          <div class="text-[13px] text-fg-body">
            <HighlightText text="session cookie, session header" query="session" />
          </div>
        </GallerySpecimen>
        <GallerySpecimen variant="no match">
          <div class="text-[13px] text-fg-body">
            <HighlightText text="session cookie after the rename" query="sid" />
          </div>
        </GallerySpecimen>
        <GallerySpecimen variant="empty query">
          <div class="text-[13px] text-fg-body">
            <HighlightText text="session cookie after the rename" query="" />
          </div>
        </GallerySpecimen>
      </div>
    </GallerySection>
  </div>
</template>
