<script setup lang="ts">
import { ArrowUpRight, ChevronRight } from '@lucide/vue'
import Button from '../ui/Button.vue'
import { ICON_PX } from '../ui/icon-metrics'
import { t } from './locales/text'
import { shortMoment } from './words'

/**
 * The bar under a run's conversation's header (`automations.md` § Where the
 * user sees them): *Automation › Failed payments every weekday › Oct 17
 * 09:00*, whose first two parts go back to the page and the automation, and
 * Move to Conversations at its end, which files the conversation in the
 * sidebar as Mail's Move to Inbox does.
 */
defineProps<{
  name: string
  /** When the run started, ISO 8601. */
  at: string
  /** The automation's time zone, whose clocks the time is read by. */
  timeZone?: string
}>()

const emit = defineEmits<{
  openPage: []
  openAutomation: []
  move: []
}>()
</script>

<template>
  <nav
    :aria-label="t('bar.label')"
    class="flex h-10 shrink-0 select-none items-center gap-1 border-y border-line-subtle bg-surface-card px-3 text-[13px] phone:h-11 phone:text-[15px]"
  >
    <button type="button" class="shrink-0 rounded px-1 text-fg-muted hover:text-fg" @click="emit('openPage')">{{ t('bar.automation') }}</button>
    <ChevronRight :size="ICON_PX.in24" class="shrink-0 text-fg-faint" />
    <button type="button" class="min-w-0 truncate rounded px-1 text-fg-muted hover:text-fg" @click="emit('openAutomation')">{{ name }}</button>
    <ChevronRight :size="ICON_PX.in24" class="shrink-0 text-fg-faint" />
    <span class="shrink-0 px-1 tabular-nums text-fg">{{ shortMoment(at, timeZone) }}</span>
    <Button size="sm" variant="ghost" class="ml-auto shrink-0" @click="emit('move')">
      <ArrowUpRight :size="ICON_PX.in24" /> <span class="phone:hidden">{{ t('bar.move') }}</span><span class="hidden phone:inline">{{ t('bar.moveShort') }}</span>
    </Button>
  </nav>
</template>
