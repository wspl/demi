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
      <aside class="flex shrink-0 flex-col gap-2 bg-surface px-2 py-2 @md:w-48 @md:px-3 @md:py-3">
        <div v-if="account" class="hidden h-9 select-none items-center gap-2 px-1.5 @md:flex">
          <span class="flex size-6 shrink-0 items-center justify-center rounded-full bg-tint-accent text-[11px] font-medium text-on-accent">
            {{ initials }}
          </span>
          <span class="flex min-w-0 flex-col leading-4">
            <span class="truncate text-chrome text-fg">{{ account.name }}</span>
            <span class="truncate text-[11px] text-fg-subtle">{{ account.plan }}</span>
          </span>
        </div>
        <nav class="flex gap-0.5 overflow-x-auto @md:flex-col" aria-label="Settings sections">
          <SidebarNavItem
            v-for="item in NAV"
            :key="item.id"
            :icon="item.icon"
            :label="item.label"
            :pressed="tab === item.id"
            class="shrink-0"
            @click="tab = item.id"
          />
        </nav>
      </aside>
      <section class="relative min-w-0 flex-1 overflow-y-auto px-8 py-6 @md:px-10 @md:py-8">
        <IconButton
          :icon="X"
          variant="ghost"
          aria-label="Close settings"
          class="absolute right-3 top-3"
          @click="emit('close')"
        />
        <slot />
      </section>
    </div>
    </div>
  </Dialog>
</template>
