<script setup lang="ts">
import { inject, onBeforeUnmount, watch } from 'vue'
import { AlarmClock } from '@lucide/vue'
import { appOverlayStore } from '../overlay/appOverlay'
import Dropdown from '../ui/Dropdown.vue'
import TaskbarButton from '../windows/TaskbarButton.vue'
import { TASKBAR_LEAVE_MS } from '../windows/taskbar-motion'
import { taskbarMenuKey } from '../windows/taskbar-menu'
import { t } from './locales/text'
import SchedulesMenu from './SchedulesMenu.vue'
import type { ConversationSchedule } from './types'

/**
 * The taskbar's Schedules button, after Agents, Running and Services and drawn
 * as they are: the alarm clock and the count of the conversation's schedules.
 * It opens them as the app's own menu upward (`SchedulesMenu`) as soon as
 * the pointer rests on it, as the system buttons beside it show theirs, and
 * the menu goes once the pointer has left both. With no schedule left, the
 * button and its menu go. It is open when `open` says so: a schedule's name
 * in the transcript opens it, and it stays until a click elsewhere.
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

let leaving: ReturnType<typeof setTimeout> | null = null

/** The button whose menu the taskbar shows; another's opening closes this one at once. */
const menuOwner = inject(taskbarMenuKey, null)
const OWN_MENU = 'schedules'
watch(open, (now) => {
  if (now && menuOwner) {
    menuOwner.value = OWN_MENU
  }
})
watch(() => menuOwner?.value, (owner) => {
  if (owner !== OWN_MENU && open.value) {
    stay()
    open.value = false
  }
})

function stay(): void {
  if (leaving) {
    clearTimeout(leaving)
    leaving = null
  }
}

function enter(): void {
  stay()
  open.value = true
}

function leave(): void {
  stay()
  leaving = setTimeout(() => {
    leaving = null
    open.value = false
  }, TASKBAR_LEAVE_MS)
}

onBeforeUnmount(stay)
</script>

<template>
  <!-- At the taskbar's end, the menu lines up with the button's end and opens leftward. -->
  <Dropdown v-if="schedules.length > 0" v-model:open="open" :overlay-store="appOverlayStore" side="top" align="end" :offset="8">
    <template #trigger="{ isOpen }">
      <!-- Resting on it opens the menu; a click there leaves it open. -->
      <TaskbarButton
        :icon="AlarmClock"
        :count="schedules.length"
        :state="isOpen ? 'front' : 'closed'"
        fixed
        :aria-label="t('name')"
        :aria-expanded="isOpen"
        aria-haspopup="dialog"
        @pointerenter="enter"
        @pointerleave="leave"
        @click.stop="enter"
      />
    </template>
    <template #content>
      <div @pointerenter="stay" @pointerleave="leave">
        <SchedulesMenu
          :schedules="schedules"
          :archived="archived"
          @pause="(id, paused) => emit('pause', id, paused)"
          @delete="(id) => emit('delete', id)"
        />
      </div>
    </template>
  </Dropdown>
</template>
