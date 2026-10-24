<script setup lang="ts">
import { computed, ref } from 'vue'
import AppSidebar from '@demicodes/web-ui/sidebar/AppSidebar.vue'
import { appShortcuts } from '@demicodes/web-ui/settings/shortcuts'
import SidebarLayout from '@demicodes/web-ui/sidebar/SidebarLayout.vue'
import type { HostMenuHost } from '@demicodes/web-ui/hosts/types'
import type { SidebarConversation } from '@demicodes/web-ui/sidebar/types'
import { GALLERY_MARKS } from '../fixtures/device-marks'
import { schedulesOfConversations, switched, wokenConversations } from '../fixtures/schedules'
import { productWould } from '../product-would'
import { demoAccount, demoConversations, demoProjects } from '../sidebar/sidebar-data'
import GalleryWokenConversation from './GalleryWokenConversation.vue'

/**
 * The app frame with a conversation schedules wake, live: its sidebar row is
 * marked with the alarm clock while it has schedules, its chip lists them, a
 * row's switch turns one off and on, and Delete in the chip's menu removes one
 * from the specimen; the last one takes the row's mark and the chip away.
 */
const props = withDefaults(defineProps<{
  /** The conversation the frame opens. */
  start?: string
  /** The frame's height; a phone's frame fills its screen. */
  fill?: boolean
}>(), { start: 'c-cloud-orders' })

const schedules = ref(schedulesOfConversations())
const conversations = ref<SidebarConversation[]>([...wokenConversations(), ...demoConversations()])
const projects = ref(demoProjects())
const activeId = ref(props.start)
const sidebarOpen = ref(false)

/** Where each woken conversation runs, which its header's Host button shows. */
const HOSTS: Record<string, HostMenuHost> = {
  'c-price-watch': { id: 'mac', name: 'MacBook Pro', kind: 'device', state: 'online', mark: GALLERY_MARKS.mac },
  'c-cloud-orders': { id: 'cloud', name: 'Cloud', kind: 'cloud', state: 'online', mark: null },
}

const listed = computed(() => conversations.value.map((each) => (
  each.id in schedules.value ? { ...each, scheduled: schedules.value[each.id]!.length > 0 } : each
)))
const title = computed(() => conversations.value.find((each) => each.id === activeId.value)?.title ?? '')

function select(id: string): void {
  activeId.value = id
  sidebarOpen.value = false
}

function pause(id: string, paused: boolean): void {
  schedules.value = {
    ...schedules.value,
    [activeId.value]: (schedules.value[activeId.value] ?? []).map((each) => (each.id === id ? switched(each, paused) : each)),
  }
}

function remove(id: string): void {
  schedules.value = {
    ...schedules.value,
    [activeId.value]: (schedules.value[activeId.value] ?? []).filter((each) => each.id !== id),
  }
}
</script>

<template>
  <div class="flex w-full overflow-hidden" :class="fill ? 'h-full' : 'gallery-frame h-[44rem]'">
    <SidebarLayout v-model:open="sidebarOpen" class="w-full" label="Schedules">
      <template #sidebar>
        <AppSidebar
          :account="demoAccount"
          :projects="projects"
          :conversations="listed"
          :active-id="activeId"
          :new-shortcut="appShortcuts().find((shortcut) => shortcut.id === 'new')?.keys"
          :search-shortcut="appShortcuts().find((shortcut) => shortcut.id === 'search')?.keys"
          :settings-shortcut="appShortcuts().find((shortcut) => shortcut.id === 'settings')?.keys"
          @select="select"
          @create="productWould('Open a New Conversation')"
          @search="productWould('Open the Search Window')"
          @add-project="productWould('Add a Project')"
          @rename="(id, title) => (conversations = conversations.map((each) => (each.id === id ? { ...each, title } : each)))"
          @pin="(ids, pinned) => (conversations = conversations.map((each) => (ids.includes(each.id) ? { ...each, pinned } : each)))"
          @archive="(ids) => (conversations = conversations.filter((each) => !ids.includes(each.id)))"
          @delete="(ids) => (conversations = conversations.filter((each) => !ids.includes(each.id)))"
          @reorder="productWould('Reorder the Sidebar')"
          @open-settings="productWould('Open Settings')"
          @hand-off-to-phone="productWould('Hand Off to Phone')"
          @retry-list="productWould('Load the Conversations Again')"
          @sign-out="productWould('Sign Out')"
        />
      </template>
      <GalleryWokenConversation
        v-if="activeId in schedules"
        :id="activeId"
        :key="activeId"
        :title="title"
        :schedules="schedules[activeId] ?? []"
        :host="HOSTS[activeId]!"
        @pause-schedule="pause"
        @delete-schedule="remove"
      />
      <div
        v-else
        class="flex min-w-0 flex-1 flex-col items-center justify-center gap-1 bg-surface text-[13px] text-fg-faint md:rounded-tl-xl"
      >
        <span>{{ title }}</span>
      </div>
    </SidebarLayout>
  </div>
</template>
