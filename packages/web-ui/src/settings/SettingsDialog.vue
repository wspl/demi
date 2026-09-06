<script setup lang="ts">
import { computed } from 'vue'
import { ChevronDown, CircleUser, Gauge, Monitor, Sparkles, X } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Dialog from '@demicodes/web-ui/ui/Dialog.vue'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuGroup from '@demicodes/web-ui/ui/MenuGroup.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import SidebarNavItem from '@demicodes/web-ui/sidebar/SidebarNavItem.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import type { SettingsAccountInfo, SettingsNavGroup, SettingsTab } from './types'

/**
 * The settings surface: one large dialog with a section rail and one page at a time.
 * The rail sits on the page surface, the page on the dialog surface, so the two read
 * as the app's own sidebar and content. Layout follows the dialog width, not the viewport.
 */
const props = withDefaults(defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  account?: SettingsAccountInfo
  /** The rail. Defaults to the product's four sections. */
  sections?: SettingsNavGroup[]
}>(), {
  sections: () => [
    {
      items: [
        { id: 'Account', label: 'Account', icon: CircleUser },
        { id: 'Devices', label: 'Devices', icon: Monitor },
        { id: 'Providers', label: 'Providers', icon: Sparkles },
        { id: 'Usage', label: 'Usage', icon: Gauge },
      ],
    },
  ],
})

const tab = defineModel<SettingsTab>('tab', { default: 'Account' })

const emit = defineEmits<{
  close: []
}>()

const items = computed(() => props.sections.flatMap((group) => group.items))
const current = computed(() => items.value.find((item) => item.id === tab.value) ?? items.value[0])
// Few sections split one row evenly; a long rail becomes a picker so nothing scrolls off.
const narrowAsRow = computed(() => items.value.length <= 4)
const initials = computed(() => props.account?.name.trim().slice(0, 1).toUpperCase() ?? '')
</script>

<template>
  <Dialog :is-open="isOpen" :overlay-store="overlayStore" size="xl" label="Settings" @close="emit('close')">
    <!-- The query container must be an ancestor of what it sizes, so it wraps the row. -->
    <div class="@container h-[36rem] max-h-full">
    <div class="flex h-full flex-col overflow-hidden @md:flex-row">
      <!-- Wide: a rail beside the page. Narrow: a compact header and the sections in one row or a picker. -->
      <aside class="flex shrink-0 flex-col bg-surface @md:w-56 @md:gap-3 @md:overflow-y-auto @md:px-3 @md:py-3">
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
        <nav class="hidden flex-col gap-3 @md:flex" aria-label="Settings sections">
          <div v-for="(group, index) in sections" :key="group.label ?? index" class="flex flex-col gap-0.5">
            <div v-if="group.label" class="select-none px-2 pb-1 text-[11px] font-medium uppercase tracking-[0.04em] text-fg-subtle">
              {{ group.label }}
            </div>
            <SidebarNavItem
              v-for="item in group.items"
              :key="item.id"
              :icon="item.icon"
              :label="item.label"
              :pressed="tab === item.id"
              @click="tab = item.id"
            />
          </div>
        </nav>
        <nav v-if="narrowAsRow" class="grid grid-cols-4 gap-1 px-2 pb-2 @md:hidden" aria-label="Settings sections">
          <button
            v-for="item in items"
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
        <div v-else class="px-2 pb-2 @md:hidden">
          <Dropdown :overlay-store="overlayStore" class="w-full [&>div]:w-full">
            <template #trigger="{ isOpen: pickerOpen }">
              <span
                role="button"
                aria-label="Settings section"
                class="flex h-8 w-full cursor-default select-none items-center gap-2 rounded-md px-2 text-chrome text-fg transition-colors duration-200 ease-out"
                :class="pickerOpen ? 'bg-active' : 'bg-hover hover:bg-active'"
              >
                <component :is="current?.icon" :size="ICON_PX.in28" class="shrink-0 text-fg-muted" />
                <span class="min-w-0 flex-1 truncate">{{ current?.label }}</span>
                <ChevronDown :size="ICON_PX.in24" class="shrink-0 text-fg-subtle" />
              </span>
            </template>
            <template #content="{ close }">
              <Menu>
                <template v-for="(group, index) in sections" :key="group.label ?? index">
                  <MenuGroup v-if="group.label" :label="group.label">
                    <MenuItem
                      v-for="item in group.items"
                      :key="item.id"
                      :icon="item.icon"
                      :label="item.label"
                      choice
                      :is-selected="tab === item.id"
                      @select="tab = item.id; close()"
                    />
                  </MenuGroup>
                  <template v-else>
                    <MenuItem
                      v-for="item in group.items"
                      :key="item.id"
                      :icon="item.icon"
                      :label="item.label"
                      choice
                      :is-selected="tab === item.id"
                      @select="tab = item.id; close()"
                    />
                  </template>
                </template>
              </Menu>
            </template>
          </Dropdown>
        </div>
      </aside>
      <section class="relative min-w-0 flex-1 overflow-y-auto px-5 py-6 @md:px-8 @md:py-8">
        <div class="absolute right-3 top-3 hidden @md:block">
          <IconButton :icon="X" variant="ghost" aria-label="Close settings" @click="emit('close')" />
        </div>
        <slot />
      </section>
    </div>
    </div>
  </Dialog>
</template>
