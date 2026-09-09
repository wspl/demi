<script setup lang="ts">
import { computed, ref } from 'vue'
import { ChevronDown, Search } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Dialog from '@demicodes/web-ui/ui/Dialog.vue'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuGroup from '@demicodes/web-ui/ui/MenuGroup.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import SidebarNavItem from '@demicodes/web-ui/sidebar/SidebarNavItem.vue'
import ScrollArea from '@demicodes/web-ui/ui/ScrollArea.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import { SETTINGS_SECTIONS } from './sections'
import type {
  SettingsAccountInfo,
  SettingsNavGroup,
  SettingsNavItem,
  SettingsTab
} from './types'

/**
 * The settings surface: one large dialog with a section rail and one page at a time.
 * The rail sits on the page surface, the page on the dialog surface, so the two read
 * as the app's own sidebar and content. Layout follows the dialog width, not the viewport.
 */
const props = withDefaults(defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  account?: SettingsAccountInfo
  /** The rail. Defaults to the product's sections. */
  sections?: SettingsNavGroup[]
}>(), {
  sections: () => SETTINGS_SECTIONS,
})

const tab = defineModel<SettingsTab>('tab', { default: 'general' })

const emit = defineEmits<{
  close: []
}>()

const items = computed(() => props.sections.flatMap((group) => group.items))

// The rail filter narrows the sections by label or keyword; Enter opens the first hit.
const query = ref('')
const matches = (item: SettingsNavItem) => {
  const q = query.value.trim().toLowerCase()
  return !q ||
    item.label.toLowerCase().includes(q) ||
    (item.keywords ?? []).some((k) => k.toLowerCase().includes(q))
}
const filteredSections = computed(() =>
  props.sections.map((group) => ({
      ...group,
      items: group.items.filter(matches)
    })).filter(
    (group) => group.items.length
  ),
)
function openFirstMatch() {
  for (const group of filteredSections.value) {
    const first = group.items.find((item) => !item.disabled)
    if (first) {
      tab.value = first.id
      return
    }
  }
}

function selectSection(id: string) {
  const item = items.value.find((entry) => entry.id === id)
  if (!item || item.disabled)
    return
  tab.value = id
}
const current = computed(
  () => items.value.find((item) => item.id === tab.value) ?? items.value[0]
)
// Few sections split one row evenly; a long rail becomes a picker so nothing scrolls off.
const narrowAsRow = computed(() => items.value.length <= 4)
const initials = computed(() => props.account?.name.trim().slice(0, 1).toUpperCase() ?? '')
</script>

<template>
  <Dialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    size="xl"
    label="Settings"
    @close="emit('close')"
  >
    <!-- The query container must be an ancestor of what it sizes, so it wraps the row. -->
    <!-- min-h-0 lets the body shrink to the panel's cap, so the rail and the page scroll
         on their own instead of the whole dialog. -->
    <div class="@container h-[36rem] min-h-0 shrink">
      <div class="flex h-full flex-col overflow-hidden @md:flex-row">
      <!-- Wide: a rail beside the page. Narrow: a compact header and the sections in one row or a picker. -->
      <!-- The account and the filter stay put; only the section list scrolls. -->
        <aside
          class="flex shrink-0 flex-col bg-surface @md:w-56 @md:gap-3 @md:px-3 @md:py-3"
        >
          <div class="flex h-11 select-none items-center pl-4 pr-12 @md:hidden">
            <span class="text-[15px] font-medium text-fg-emphasis">Settings</span>
          </div>
          <div
            v-if="account"
            class="hidden h-9 select-none items-center gap-2 px-1.5 @md:flex"
          >
            <span
              class="flex size-6 shrink-0 items-center justify-center rounded-full bg-tint-accent text-[11px] font-medium text-on-accent"
            >
            {{ initials }}
            </span>
            <span class="min-w-0 truncate text-chrome text-fg">{{ account.name }}</span>
          </div>
          <div class="hidden @md:block">
            <TextInput
              v-model="query"
              placeholder="Filter settings"
              aria-label="Filter settings"
              @keydown.enter="openFirstMatch"
            >
              <template #prefix><Search :size="ICON_PX.in24" /></template>
            </TextInput>
          </div>
        <!-- The list spans the rail edge to edge; its thumb is drawn over the content, taking no room. -->
          <ScrollArea
            class="hidden min-h-0 flex-1 @md:-mx-3 @md:block"
            viewport-class="@md:px-3"
          >
            <nav class="flex flex-col gap-3" aria-label="Settings sections">
              <div
                v-if="!filteredSections.length"
                class="select-none px-2 py-3 text-[12px] text-fg-subtle"
              >Nothing matches.</div>
              <div
                v-for="(group, index) in filteredSections"
                :key="group.label ?? index"
                class="flex flex-col gap-0.5"
              >
                <div
                  v-if="group.label"
                  class="select-none px-2 pb-1 text-[11px] font-medium uppercase tracking-[0.04em] text-fg-subtle"
                >
              {{ group.label }}
                </div>
                <SidebarNavItem
                  v-for="item in group.items"
                  :key="item.id"
                  :icon="item.icon"
                  :label="item.label"
                  :pressed="tab === item.id"
                  :disabled="item.disabled"
                  :disabled-reason="item.disabledReason"
                  @click="selectSection(item.id)"
                />
              </div>
            </nav>
          </ScrollArea>
          <nav
            v-if="narrowAsRow"
            class="grid grid-cols-4 gap-1 px-2 pb-2 @md:hidden"
            aria-label="Settings sections"
          >
            <Tooltip
              v-for="item in items"
              :key="item.id"
              :content="item.disabledReason"
              :disabled="!item.disabled || !item.disabledReason"
              :open-delay-ms="80"
            >
              <button
                type="button"
                class="flex h-7 w-full cursor-default select-none items-center justify-center rounded-md text-[12px] transition-colors duration-200 ease-out"
                :class="item.disabled
                ? 'cursor-not-allowed text-fg-faint'
                : tab === item.id ? 'bg-active text-fg-emphasis' : 'text-fg-muted hover:bg-hover hover:text-fg'"
                :aria-pressed="tab === item.id"
                :aria-disabled="item.disabled || undefined"
                @click="selectSection(item.id)"
              >
              {{ item.label }}
              </button>
            </Tooltip>
          </nav>
          <div v-else class="px-2 pb-2 @md:hidden">
            <Dropdown
              :overlay-store="overlayStore"
              class="w-full [&>div]:w-full"
            >
              <template #trigger="{ isOpen: pickerOpen }">
                <span
                  role="button"
                  aria-label="Settings section"
                  class="flex h-8 w-full cursor-default select-none items-center gap-2 rounded-md px-2 text-chrome text-fg transition-colors duration-200 ease-out"
                  :class="pickerOpen ? 'bg-active' : 'bg-hover hover:bg-active'"
                >
                  <component
                    :is="current?.icon"
                    :size="ICON_PX.in28"
                    class="shrink-0 text-fg-muted"
                  />
                  <span class="min-w-0 flex-1 truncate">{{ current?.label }}</span>
                  <ChevronDown
                    :size="ICON_PX.in24"
                    class="shrink-0 text-fg-subtle"
                  />
                </span>
              </template>
              <template #content="{ close }">
                <Menu>
                  <template
                    v-for="(group, index) in sections"
                    :key="group.label ?? index"
                  >
                    <MenuGroup v-if="group.label" :label="group.label">
                      <MenuItem
                        v-for="item in group.items"
                        :key="item.id"
                        :icon="item.icon"
                        :label="item.label"
                        choice
                        :is-selected="tab === item.id"
                        :disabled="item.disabled"
                        :disabled-reason="item.disabledReason"
                        @select="selectSection(item.id); close()"
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
                        :disabled="item.disabled"
                        :disabled-reason="item.disabledReason"
                        @select="selectSection(item.id); close()"
                      />
                    </template>
                  </template>
                </Menu>
              </template>
            </Dropdown>
          </div>
        </aside>
        <ScrollArea
          class="relative min-w-0 flex-1"
          viewport-class="flex flex-col px-5 py-6 @md:px-8 @md:py-8"
        >
          <slot />
        </ScrollArea>
      </div>
    </div>
  </Dialog>
</template>
