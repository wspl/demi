<script setup lang="ts">
import { computed } from 'vue'
import { CircleUser, Gauge, Monitor, Sparkles, X } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Dialog from '@demicodes/web-ui/ui/Dialog.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import SidebarNavItem from '@demicodes/web-ui/sidebar/SidebarNavItem.vue'
import type { SettingsAccountInfo, SettingsNavItem, SettingsTab } from './types'

/**
 * The settings surface: one large dialog with a section rail and one page at a time.
 * The rail sits on the page surface, the page on the dialog surface, so the two read
 * as the app's own sidebar and content. Layout follows the dialog width, not the viewport.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  account?: SettingsAccountInfo
}>()

const tab = defineModel<SettingsTab>('tab', { default: 'Account' })

const emit = defineEmits<{
  close: []
}>()

const NAV: SettingsNavItem[] = [
  { id: 'Account', label: 'Account', icon: CircleUser },
  { id: 'Devices', label: 'Devices', icon: Monitor },
  { id: 'Providers', label: 'Providers', icon: Sparkles },
  { id: 'Usage', label: 'Usage', icon: Gauge },
]

const initials = computed(() => props.account?.name.trim().slice(0, 1).toUpperCase() ?? '')
</script>

<template>
  <Dialog :is-open="isOpen" :overlay-store="overlayStore" size="xl" label="Settings" @close="emit('close')">
    <!-- The query container must be an ancestor of what it sizes, so it wraps the row. -->
    <div class="@container h-[36rem] max-h-full">
    <div class="flex h-full flex-col overflow-hidden @md:flex-row">
      <!-- Wide: a rail beside the page. Narrow: a compact header and an evenly split section row. -->
      <aside class="flex shrink-0 flex-col bg-surface @md:w-48 @md:gap-2 @md:px-3 @md:py-3">
        <div class="flex h-11 select-none items-center justify-between pl-4 pr-2 @md:hidden">
          <span class="text-[15px] font-medium text-fg-emphasis">Settings</span>
          <IconButton :icon="X" variant="ghost" aria-label="Close settings" @click="emit('close')" />
        </div>
        <div v-if="account" class="hidden h-9 select-none items-center gap-2 px-1.5 @md:flex">
          <span class="flex size-6 shrink-0 items-center justify-center rounded-full bg-tint-accent text-[11px] font-medium text-on-accent">
            {{ initials }}
          </span>
          <span class="flex min-w-0 flex-col leading-4">
            <span class="truncate text-chrome text-fg">{{ account.name }}</span>
            <span class="truncate text-[11px] text-fg-subtle">{{ account.plan }}</span>
          </span>
        </div>
        <nav class="hidden flex-col gap-0.5 @md:flex" aria-label="Settings sections">
          <SidebarNavItem
            v-for="item in NAV"
            :key="item.id"
            :icon="item.icon"
            :label="item.label"
            :pressed="tab === item.id"
            @click="tab = item.id"
          />
        </nav>
        <nav class="grid grid-cols-4 gap-1 px-2 pb-2 @md:hidden" aria-label="Settings sections">
          <button
            v-for="item in NAV"
            :key="item.id"
            type="button"
            class="flex h-7 cursor-default select-none items-center justify-center rounded-md text-[12px] transition-colors duration-200 ease-out"
            :class="tab === item.id ? 'bg-active text-fg-emphasis' : 'text-fg-muted hover:bg-hover hover:text-fg'"
            :aria-pressed="tab === item.id"
            @click="tab = item.id"
          >
            {{ item.label }}
          </button>
        </nav>
      </aside>
      <section class="relative min-w-0 flex-1 overflow-y-auto px-5 py-6 @md:px-10 @md:py-8">
        <div class="absolute right-3 top-3 hidden @md:block">
          <IconButton :icon="X" variant="ghost" aria-label="Close settings" @click="emit('close')" />
        </div>
        <slot />
      </section>
    </div>
    </div>
  </Dialog>
</template>
