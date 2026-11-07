<script setup lang="ts">
import { reactive, ref } from 'vue'
import AssistantTextBlock from '@demicodes/web-ui/agent/blocks/AssistantTextBlock.vue'
import SchedulesButton from '@demicodes/web-ui/schedules/SchedulesButton.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import { gallerySchedules, switched } from '../fixtures/schedules'
import { gallerySubagents } from '../fixtures/subagents'
import { previewService, runningRunCommand } from '../fixtures/terminals'
import GalleryOwnWindows from './GalleryOwnWindows.vue'

/**
 * The taskbar's system area with Schedules after Agents, Running and
 * Services, and the menu it opens above them. A row's switch turns it off and
 * on; Delete removes the row and leaves the menu open; once none is left the
 * button and the menu go, as the product's would. Archive shows the menu of an
 * archived conversation.
 */
const schedules = reactive(gallerySchedules())
const archived = ref(false)
const agents = reactive(gallerySubagents())
const terminals = [runningRunCommand(), previewService()]
const createdAt = new Date(Date.now() - 60_000).toISOString()

function remove(id: string): void {
  const at = schedules.findIndex((schedule) => schedule.id === id)
  if (at >= 0) {
    schedules.splice(at, 1)
  }
}

function pause(id: string, paused: boolean): void {
  const at = schedules.findIndex((schedule) => schedule.id === id)
  if (at >= 0) {
    schedules.splice(at, 1, switched(schedules[at]!, paused))
  }
}

/** Stands in for the agent making its schedules again with `demi schedule create`. */
function restore(): void {
  schedules.splice(0, schedules.length, ...gallerySchedules())
}
</script>

<template>
  <div class="flex flex-col gap-3">
    <div class="flex flex-wrap gap-2">
      <Button variant="ghost" size="sm" @click="restore">Schedule Again</Button>
      <Button variant="ghost" size="sm" @click="archived = !archived">{{ archived ? 'Restore Conversation' : 'Archive Conversation' }}</Button>
    </div>
    <div data-region class="gallery-frame h-[40rem] w-full overflow-hidden bg-surface">
      <GalleryOwnWindows :agents="agents" :terminals="terminals">
        <div class="flex h-full flex-col justify-end p-3">
          <AssistantTextBlock content="I’ll wake this conversation every Monday at 10:00 for the report, and when the CI on main changes." :created-at="createdAt" />
        </div>
        <template #system>
          <SchedulesButton :schedules="schedules" :archived="archived" @pause="pause" @delete="remove" />
        </template>
      </GalleryOwnWindows>
    </div>
  </div>
</template>
