<script setup lang="ts">
import { Plus, TriangleAlert } from '@lucide/vue'
import Button from '../ui/Button.vue'
import StatusDot from '../ui/StatusDot.vue'
import { ICON_PX } from '../ui/icon-metrics'
import SettingsGroup from '../settings/SettingsGroup.vue'
import SettingsRow from '../settings/SettingsRow.vue'
import { t } from './locales/text'
import type { Automation, AutomationConversation, AutomationHost } from './types'
import { lastRunWords, targetWords, triggerWords } from './words'

/**
 * The Automations page: each automation's name, its trigger and target in
 * words, and its last run, with a dot while that run notified and its
 * conversation is unread, and a warning when it failed; a paused one is dimmed
 * and says so. New Automation creates one at once, which the host opens.
 */
defineProps<{
  automations: readonly Automation[]
  hosts: readonly AutomationHost[]
  conversations: readonly AutomationConversation[]
  /** The moment the last runs are said against. */
  now: Date | number
}>()

const emit = defineEmits<{
  open: [id: string]
  create: []
}>()
</script>

<template>
  <div class="mx-auto flex w-full max-w-3xl flex-col gap-6">
    <header class="flex select-none items-start gap-4">
      <div class="min-w-0 flex-1">
        <h2 class="text-[20px] font-medium leading-7 text-fg-emphasis phone:hidden">{{ t('page.title') }}</h2>
        <p class="mt-1 text-[13px] leading-5 text-fg-muted phone:mt-0 phone:text-[15px]">{{ t('page.description') }}</p>
      </div>
      <!-- On a phone the top bar's + creates one instead. -->
      <Button size="sm" class="shrink-0 phone:hidden" @click="emit('create')">
        <Plus :size="ICON_PX.in24" /> {{ t('page.new') }}
      </Button>
    </header>
    <div
      v-if="automations.length === 0"
      class="flex select-none flex-col items-center gap-1 rounded-xl border border-dashed border-line px-6 py-12 text-center"
    >
      <p class="text-[15px] font-medium text-fg">{{ t('page.empty') }}</p>
      <p class="text-[13px] text-fg-muted">{{ t('page.emptyHint') }}</p>
    </div>
    <SettingsGroup v-else>
      <!-- On a phone the last run is a line under the trigger, as Mail's rows stack; on a desktop it stands at the row's end. -->
      <SettingsRow
        v-for="automation in automations"
        :key="automation.id"
        :label="automation.name"
        :muted="automation.paused"
        :statuses="automation.paused ? [{ label: t('page.paused') }] : undefined"
        navigable
        @click="emit('open', automation.id)"
      >
        <template #description>
          <span class="block">{{ triggerWords(automation) }} · {{ targetWords(automation, hosts, conversations) }}</span>
          <span class="mt-0.5 hidden items-center gap-1.5 phone:flex">
            {{ lastRunWords(automation.runs[0], now) }}
            <StatusDot v-if="automation.runs[0]?.unread" tone="accent" :label="t('page.unread')" />
            <TriangleAlert
              v-else-if="automation.runs[0]?.state === 'failed'"
              :size="ICON_PX.in24"
              class="text-on-warning"
              :aria-label="t('page.failed')"
            />
          </span>
        </template>
        <span class="flex shrink-0 items-center gap-2 text-[13px] text-fg-subtle phone:hidden">
          {{ lastRunWords(automation.runs[0], now) }}
          <StatusDot v-if="automation.runs[0]?.unread" tone="accent" :label="t('page.unread')" />
          <TriangleAlert
            v-else-if="automation.runs[0]?.state === 'failed'"
            :size="ICON_PX.in24"
            class="text-on-warning"
            :aria-label="t('page.failed')"
          />
        </span>
      </SettingsRow>
    </SettingsGroup>
  </div>
</template>
