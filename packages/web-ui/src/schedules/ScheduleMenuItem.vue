<script setup lang="ts">
import { computed } from 'vue'
import { Trash2 } from '@lucide/vue'
import IconButton from '../ui/IconButton.vue'
import MenuItem from '../ui/MenuItem.vue'
import Switch from '../ui/Switch.vue'
import Tooltip from '../ui/Tooltip.vue'
import { usePhone } from '../ui/phone'
import { t } from './locales/text'
import { REVEAL_PX, useSwipeDelete } from './swipe-delete'
import { triggerIcon } from './trigger-icon'
import type { ConversationSchedule } from './types'
import { lastWakeReason, triggerWords, wakeWords } from './words'

/**
 * A schedule as a row of the app's menu, taller and roomier than a plain menu's
 * row. Its icon is its trigger's kind, and a switch at its end turns it on and
 * off, as the Clock app's alarms are.
 *
 * - On a computer a row is one line, as wide as its text needs. The name, the
 *   trigger after it and, in the warning color, why its last wake failed or was
 *   skipped stand at the start; the time from now stands before the switch in
 *   the menu's value column, so the times of all rows line up. A row that is
 *   off is faded and has no time. Over the pointer's row, Delete takes the
 *   time's place, as Mail shows a row's actions over its date.
 * - On a phone the name and the time share the first line, as Mail's rows do;
 *   the trigger is the second line and the reason the third, and the switch
 *   stands at the end. Swiping the row to the left lays a Delete bare.
 * - In an archived conversation, whose schedules wake nothing, every row is
 *   faded, with no time, its switch disabled and no Delete: the menu only lists
 *   them.
 */
const props = defineProps<{
  schedule: ConversationSchedule
  now: Date | number
  /** The conversation is archived. */
  archived?: boolean
}>()

const emit = defineEmits<{
  /** The switch turned the schedule off, or with `false` on. */
  pause: [paused: boolean]
  delete: []
}>()

const phone = usePhone()
const trigger = computed(() => triggerWords(props.schedule, props.now))
const wake = computed(() => (props.archived ? null : wakeWords(props.schedule, props.now)))
const reason = computed(() => lastWakeReason(props.schedule))
const { offset, dragging, start, tap } = useSwipeDelete()
</script>

<template>
  <!--
    The row's height and side room are the menu row's own variables, set larger;
    a phone's row grows with its lines.
  -->
  <MenuItem
    choice
    class="schedule-menu-row group/row relative [--menu-row-h:2.125rem]! [--menu-row-px:0.75rem]! phone:[--menu-row-h:auto]! phone:py-1.5"
    :class="dragging ? '' : 'transition-transform duration-200 ease-out'"
    :style="{ transform: `translateX(${offset}px)` }"
    :icon="triggerIcon(schedule.trigger)"
    :value="phone ? undefined : wake ?? undefined"
    :faded="schedule.paused || archived"
    :indicator="reason ? 'warning' : undefined"
    :indicator-label="reason ?? undefined"
    @touchstart="archived ? undefined : start($event)"
    @click="tap"
  >
    <span v-if="phone" class="flex min-w-0 flex-1 flex-col gap-0.5 py-0.5">
      <span class="flex min-w-0 items-baseline gap-2">
        <span class="min-w-0 flex-1 truncate">{{ schedule.name }}</span>
        <span v-if="wake" class="shrink-0 text-detail tabular-nums text-fg-subtle">{{ wake }}</span>
      </span>
      <span class="truncate text-detail text-fg-subtle">{{ trigger }}</span>
      <span v-if="reason" class="text-detail text-on-warning">{{ reason }}</span>
    </span>
    <span v-else class="flex min-w-0 items-baseline gap-2">
      <span class="shrink-0">{{ schedule.name }}</span>
      <span class="min-w-0 truncate text-detail text-fg-subtle">{{ trigger }}</span>
      <span v-if="reason" class="min-w-0 truncate text-detail text-on-warning">{{ reason }}</span>
    </span>
    <template #actions>
      <span class="flex items-center" :class="phone ? 'pl-2' : 'relative'" @click.stop>
        <!--
          Delete stands over the time's place, just before the switch, over the
          pointer's row.
        -->
        <Tooltip v-if="!phone && !archived" :content="t('delete')">
          <IconButton
            :icon="Trash2"
            size="xs"
            variant="ghost"
            :aria-label="t('delete')"
            class="absolute right-full top-1/2 mr-2 -translate-y-1/2 opacity-0 transition-opacity group-hover/row:opacity-100 focus-visible:opacity-100"
            @click="emit('delete')"
          />
        </Tooltip>
        <Switch
          :model-value="!schedule.paused"
          size="sm"
          :disabled="archived"
          :aria-label="schedule.name"
          class="ml-3"
          @update:model-value="(on: boolean) => emit('pause', !on)"
        />
        <!-- The swipe's Delete waits just outside the row's end and comes into view as the row slides. -->
        <button
          v-if="phone && !archived"
          type="button"
          tabindex="-1"
          aria-hidden="true"
          class="absolute left-full top-0 hidden h-full cursor-default items-center justify-center bg-tint-danger-strong text-on-danger"
          :class="offset < 0 ? 'phone:flex' : ''"
          :style="{ width: `${REVEAL_PX}px` }"
          @click="emit('delete')"
        >
          {{ t('delete') }}
        </button>
      </span>
    </template>
  </MenuItem>
</template>

<style>
/* Over the pointer's row the time gives way to Delete, and keeps its room. */
.schedule-menu-row:hover .menu-cell-value,
.schedule-menu-row:focus-within .menu-cell-value {
  visibility: hidden;
}

/* The time's column keeps room for Delete, which takes its place. */
.schedule-menu-row .menu-cell-value {
  min-width: 2.5rem;
}
</style>
