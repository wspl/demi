<script setup lang="ts">
import { useNow } from '@vueuse/core'
import { type ComponentPublicInstance, nextTick, onMounted, ref, useTemplateRef } from 'vue'
import Menu from '../ui/Menu.vue'
import { usePhone } from '../ui/phone'
import ScheduleMenuItem from './ScheduleMenuItem.vue'
import type { ConversationSchedule } from './types'

/**
 * The schedules that wake a conversation, one row each (`ScheduleMenuItem`):
 * the user pauses, resumes and deletes them here, and the agent does the
 * rest. Pause and Delete ask nothing, as deleting a queued message does not,
 * and leave the menu open.
 *
 * It opens as wide as its widest row needs, between 20rem and 40rem, and
 * keeps that width while it stays open, as a browser's tabs keep theirs while
 * one is closed: a row turned on gains its time, and a time grows as it
 * drifts, by narrowing the text before them, never by moving the menu's edge.
 * On a phone it spans its sheet.
 */
defineProps<{
  schedules: readonly ConversationSchedule[]
  /** The conversation is archived: its schedules wake nothing, and the menu only lists them. */
  archived?: boolean
}>()

const emit = defineEmits<{
  /** Pause on a row, or with `false` Resume. */
  pause: [id: string, paused: boolean]
  /** Delete on a row. */
  delete: [id: string]
}>()

// The times from now drift while the menu stays open.
const now = useNow({ interval: 30_000 })

const phone = usePhone()
const menu = useTemplateRef<ComponentPublicInstance>('menu')
/** The width the menu opened at, in pixels; none before it is measured, or on a phone. */
const openWidth = ref<number | null>(null)
onMounted(async () => {
  if (phone.value) {
    return
  }
  await nextTick()
  const element: unknown = menu.value?.$el
  // The computed width, not the drawn box, which the opening animation scales.
  if (element instanceof HTMLElement) {
    openWidth.value = Number.parseFloat(getComputedStyle(element).width)
  }
})
</script>

<template>
  <Menu
    ref="menu"
    :class="phone ? '' : 'min-w-[20rem] max-w-[40rem]'"
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
