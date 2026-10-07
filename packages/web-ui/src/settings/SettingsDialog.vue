<script setup lang="ts">
import { computed, onBeforeUnmount, provide, ref, shallowRef, watch } from 'vue'
import { useElementSize } from '@vueuse/core'
import { accountDisplayName, accountInitial } from '../auth/account-display'
import { ChevronLeft, Search } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Dialog from '@demicodes/web-ui/ui/Dialog.vue'
import SidebarNavItem from '@demicodes/web-ui/sidebar/SidebarNavItem.vue'
import ScrollArea from '@demicodes/web-ui/ui/ScrollArea.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import { useTouchOnly } from '../ui/touch-only'
import { SETTINGS_SECTIONS } from './sections'
import { filterSettings, firstMatch } from './settings-filter'
import { highlightFound } from '../ui/found-highlight'
import { settingsLevelKey, type SettingsLevel } from './navigation'
import type {
  SettingsAccountInfo,
  SettingsNavGroup,
  SettingsTab
} from './types'

/**
 * The settings surface: one large dialog with a section rail and one page at a time.
 * The rail sits on the page surface, the page on the dialog surface, so the two read
 * as the app's own sidebar and content. Layout follows the dialog width, not the viewport.
 * On a narrow screen the dialog fills the window (Dialog's xl size) and reads as iOS
 * Settings does: the rail is a list of the sections, and a section opens as a page of
 * its own with a back button to the list.
 *
 * The rail's filter finds sections and the settings they hold (`SettingsEntry`); a
 * setting found opens its section with its row highlighted. Escape in the filter
 * clears it before it closes anything.
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

/**
 * The open section; null for none, which a narrow dialog shows as the list of
 * sections and a wide one as its first section.
 */
const tab = defineModel<SettingsTab | null>('tab', { default: null })

const emit = defineEmits<{
  close: []
}>()

const touchOnly = useTouchOnly()
/** The sections this device lists: a touch phone has no keyboard to set up. */
const shownSections = computed(() =>
  props.sections
    .map((group) => ({ ...group, items: group.items.filter((item) => !item.keyboard || !touchOnly.value) }))
    .filter((group) => group.items.length),
)
const items = computed(() => shownSections.value.flatMap((group) => group.items))
const firstSection = computed(() => items.value.find((item) => !item.disabled)?.id ?? null)
/** The section a wide dialog shows: the open one, or its first. */
const shown = computed(() => tab.value ?? firstSection.value)

const container = ref<HTMLElement>()
const { width } = useElementSize(container)
/** Narrow as the container queries below are (28rem): the list rows take a finger's height. */
const narrow = computed(() => width.value > 0 && width.value < 448)

/** A level the open page opened inside itself, which the narrow bar goes back from first. */
// Shallow: a split takes its level off only while it is the one it put on, by identity.
const level = shallowRef<SettingsLevel | null>(null)
provide(settingsLevelKey, level)

/** The narrow bar's back button: out of a page's inner level, else to the list of sections. */
function back(): void {
  if (level.value) {
    level.value.back()
    return
  }
  tab.value = null
}

const query = ref('')
const filtered = computed(() => filterSettings(shownSections.value, query.value))

/** The page, where a setting the filter opened is looked for. */
const page = ref<HTMLElement>()
let finding: AbortController | null = null

function open(id: string, setting: string | null = null): void {
  const item = items.value.find((entry) => entry.id === id)
  if (!item || item.disabled)
    return
  tab.value = id
  finding?.abort()
  finding = null
  if (setting && page.value) {
    finding = new AbortController()
    highlightFound(page.value, `[data-setting="${CSS.escape(setting)}"]`, finding.signal)
  }
}

function openFirstMatch(): void {
  const match = firstMatch(filtered.value)
  if (match)
    open(match.section, match.setting)
}

/** Escape with text in the filter clears it; the next Escape closes the dialog. */
function clearFilter(event: KeyboardEvent): void {
  if (!query.value)
    return
  event.preventDefault()
  query.value = ''
}

watch(() => props.isOpen, (isOpen) => {
  if (!isOpen) {
    finding?.abort()
    finding = null
  }
})
onBeforeUnmount(() => finding?.abort())

const displayName = computed(() =>
  accountDisplayName(props.account?.name ?? '', props.account?.email)
)
const initials = computed(() =>
  accountInitial(props.account?.name ?? '', props.account?.email)
)
</script>

<template>
  <Dialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    size="xl"
    label="Settings"
    :scroll-content="false"
    close-over-content
    @close="emit('close')"
  >
    <!-- The query container must be an ancestor of what it sizes, so it wraps the row. -->
    <!-- The rail and the page scroll on their own. min-h-0 lets the body shrink to a
         floating panel's cap; grow lets it fill a panel that fills a narrow window. -->
    <div ref="container" class="@container h-[36rem] min-h-0 shrink grow">
      <div class="flex h-full flex-col overflow-hidden @md:flex-row">
        <!-- Wide: a rail beside the page. Narrow: the list of sections, until one opens. -->
        <!-- The account and the filter stay put; only the section list scrolls. -->
        <aside
          class="min-h-0 shrink-0 flex-col gap-3 bg-surface px-3 pb-3 @md:flex @md:w-56 @md:pt-3"
          :class="tab === null ? 'flex flex-1 @md:flex-none' : 'hidden'"
        >
          <div class="-mx-3 -mb-3 flex h-11 shrink-0 select-none items-center pl-4 pr-12 @md:hidden">
            <span class="text-[15px] font-medium text-fg-emphasis">Settings</span>
          </div>
          <div
            v-if="account"
            class="flex h-9 shrink-0 select-none items-center gap-2 px-1.5"
          >
            <span
              class="flex size-6 shrink-0 items-center justify-center rounded-full bg-tint-accent text-[11px] font-medium text-on-accent"
            >
              {{ initials }}
            </span>
            <span class="min-w-0 truncate text-chrome text-fg">{{ displayName }}</span>
          </div>
          <TextInput
            v-model="query"
            class="shrink-0"
            :size="narrow ? 'lg' : 'md'"
            placeholder="Filter settings"
            aria-label="Filter settings"
            @keydown.enter="openFirstMatch"
            @keydown.escape="clearFilter"
          >
            <template #prefix><Search :size="ICON_PX.in24" /></template>
          </TextInput>
          <!-- The list spans the rail edge to edge; its thumb is drawn over the content, taking no room. -->
          <ScrollArea
            class="-mx-3 min-h-0 flex-1"
            viewport-class="px-3"
          >
            <nav class="flex flex-col gap-3" aria-label="Settings sections">
              <div
                v-if="!filtered.length"
                class="select-none px-2 py-3 text-[12px] text-fg-subtle"
              >Nothing matches.</div>
              <div
                v-for="(group, index) in filtered"
                :key="group.label ?? index"
                class="flex flex-col gap-0.5"
              >
                <div
                  v-if="group.label"
                  class="select-none px-2 pb-1 text-[11px] font-medium uppercase tracking-[0.04em] text-fg-subtle"
                >
                  {{ group.label }}
                </div>
                <template v-for="match in group.matches" :key="match.item.id">
                  <SidebarNavItem
                    :icon="match.item.icon"
                    :label="match.item.label"
                    :size="narrow ? 'lg' : 'md'"
                    :pressed="!narrow && shown === match.item.id"
                    :disabled="match.item.disabled"
                    :disabled-reason="match.item.disabledReason"
                    @click="open(match.item.id)"
                  />
                  <!-- The settings the filter found, under their section, each opening it on its row. -->
                  <div
                    v-for="setting in match.settings"
                    :key="setting.label"
                    class="pl-6"
                  >
                    <SidebarNavItem
                      :label="setting.label"
                      :size="narrow ? 'lg' : 'md'"
                      :disabled="match.item.disabled"
                      :disabled-reason="match.item.disabledReason"
                      @click="open(match.item.id, setting.label)"
                    />
                  </div>
                </template>
              </div>
            </nav>
          </ScrollArea>
        </aside>
        <section
          class="relative min-h-0 min-w-0 flex-1 flex-col @md:flex"
          :class="tab === null ? 'hidden' : 'flex'"
        >
          <div class="flex h-11 shrink-0 select-none items-center pl-2 pr-12 @md:hidden">
            <Button variant="ghost" size="sm" @click="back">
              <ChevronLeft :size="ICON_PX.in24" />
              {{ level?.label ?? 'Settings' }}
            </Button>
          </div>
          <ScrollArea
            class="min-h-0 flex-1"
            viewport-class="flex flex-col px-5 pb-6 pt-2 @md:px-8 @md:py-8"
          >
            <div ref="page" class="contents">
              <slot :section="shown" />
            </div>
          </ScrollArea>
        </section>
      </div>
    </div>
  </Dialog>
</template>
