<script setup lang="ts">
import { AlarmClock } from '@lucide/vue'
import { useNow } from '@vueuse/core'
import { type ComponentPublicInstance, nextTick, ref, useTemplateRef, watch } from 'vue'
import SessionDockChip from '../agent/SessionDockChip.vue'
import { appOverlayStore } from '../overlay/appOverlay'
import Dropdown from '../ui/Dropdown.vue'
import Menu from '../ui/Menu.vue'
import { useIconPx } from '../ui/icon-metrics'
import { usePhone } from '../ui/phone'
import { t } from './locales/text'
import ScheduleMenuItem from './ScheduleMenuItem.vue'
import type { ConversationSchedule } from './types'

/**
 * The chip above the composer of a conversation that has schedules, beside
 * Running and Agents: the alarm clock and *3 Scheduled*. It opens the
 * schedules as the app's own menu upward, a sheet on a phone. A row is one
 * schedule (`ScheduleMenuItem`); the user pauses, resumes and deletes it there,
 * and the agent does the rest. Pause and Delete ask nothing, as deleting a
 * queued message does not, and leave the menu open. With no schedule left, the
 * chip and its menu go.
 *
 * The menu opens as wide as its widest row needs, between 20rem and 40rem, and
 * keeps that width while it stays open, as a browser's tabs keep theirs while
 * one is closed: a row turned on gains its time, and a time grows as it
 * drifts, by narrowing the text before them, never by moving the menu's edge.
 * It is open when `open` says so: a schedule's name in the transcript opens it.
 */
defineProps<{
  schedules: readonly ConversationSchedule[]
  /** The conversation is archived: its schedules wake nothing, and the menu only lists them. */
  archived?: boolean
}>()

const open = defineModel<boolean>('open', { default: false })

const emit = defineEmits<{
  /** Pause on a row, or with `false` Resume. */
  pause: [id: string, paused: boolean]
  /** Delete on a row. */
  delete: [id: string]
}>()

const icons = useIconPx()
// The times from now drift while the menu stays open.
const now = useNow({ interval: 30_000 })

const phone = usePhone()
const menu = useTemplateRef<ComponentPublicInstance>('menu')
/** The width the menu opened at, in pixels; none while it is closed, before it is measured, or on a phone, whose sheet spans the screen. */
const openWidth = ref<number | null>(null)
watch([open, phone], async ([isOpen, isPhone]) => {
  openWidth.value = null
  if (!isOpen || isPhone)
    return
  await nextTick()
  const element: unknown = menu.value?.$el
  // The computed width, not the drawn box, which the opening animation scales.
  if (element instanceof HTMLElement && open.value)
    openWidth.value = Number.parseFloat(getComputedStyle(element).width)
}, { immediate: true })
</script>

<template>
  <Dropdown v-if="schedules.length > 0" v-model:open="open" :overlay-store="appOverlayStore" side="top" :offset="8">
    <template #trigger="{ isOpen }">
      <SessionDockChip
        :aria-expanded="isOpen"
        aria-haspopup="dialog"
        data-schedules-chip
      >
        <AlarmClock :size="icons.chip" />
        {{ t('chip', { count: schedules.length }, schedules.length) }}
      </SessionDockChip>
    </template>
    <template #content>
      <Menu
        ref="menu"
        class="min-w-[20rem] max-w-[40rem]"
        :style="openWidth === null ? undefined : { width: `${openWidth}px` }"
      >
        <ScheduleMenuItem
          v-for="schedule in schedules"
          :key="schedule.id"
          :schedule="schedule"
          :now="now"
          :archived="archived"
          @pause="(paused) => emit('pause', schedule.id, paused)"
          @delete="emit('delete', schedule.id)"
        />
      </Menu>
    </template>
  </Dropdown>
</template>
