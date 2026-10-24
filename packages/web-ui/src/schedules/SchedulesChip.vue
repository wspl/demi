<script setup lang="ts">
import { AlarmClock } from '@lucide/vue'
import { useNow } from '@vueuse/core'
import SessionDockChip from '../agent/SessionDockChip.vue'
import { appOverlayStore } from '../overlay/appOverlay'
import Dropdown from '../ui/Dropdown.vue'
import Menu from '../ui/Menu.vue'
import { useIconPx } from '../ui/icon-metrics'
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
 * The menu is as wide as its widest row needs, between 20rem and 40rem. It is
 * open when `open` says so: a schedule's name in the transcript opens it.
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
      <Menu class="min-w-[20rem] max-w-[40rem]">
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
