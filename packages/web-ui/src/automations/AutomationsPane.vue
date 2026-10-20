<script setup lang="ts">
import { ChevronLeft, PanelLeft, Plus } from '@lucide/vue'
import Button from '../ui/Button.vue'
import { ICON_PX } from '../ui/icon-metrics'
import IconButton from '../ui/IconButton.vue'
import ScrollArea from '../ui/ScrollArea.vue'
import { usePhone } from '../ui/phone'
import { useSidebarDrawer } from '../sidebar/sidebar-drawer'
import { t } from './locales/text'

/**
 * The main area while it shows the Automations page or one automation: the
 * raised pane a conversation stands in, scrolling as a whole. On a phone it
 * carries the one top bar, with the sidebar's button and the page's title, as a
 * conversation's does; on a page inside it, an automation, the bar goes back
 * instead, as an iOS navigation bar does, and the page names itself. At the
 * top, New Automation is a + at the bar's end, as Reminders and Mail put
 * theirs.
 */
defineProps<{
  /** The bar's title on a phone. */
  title: string
  /** The page the bar goes back to on a phone; none at the top. */
  back?: string
  /** The + at the bar's end on a phone, by its name; none without it. */
  add?: string
}>()

const emit = defineEmits<{
  back: []
  add: []
}>()

const phone = usePhone()
const drawer = useSidebarDrawer()
</script>

<template>
  <section class="flex min-h-0 flex-1 flex-col overflow-hidden bg-surface md:rounded-tl-xl">
    <header
      v-if="phone"
      class="flex h-[calc(2.75rem+var(--safe-top))] shrink-0 select-none items-center gap-1 px-1.5 pt-[var(--safe-top)]"
    >
      <Button v-if="back" variant="ghost" class="shrink-0" @click="emit('back')">
        <ChevronLeft :size="ICON_PX.in24" /> {{ back }}
      </Button>
      <IconButton
        v-else-if="drawer"
        :icon="PanelLeft"
        variant="ghost"
        :aria-label="t('page.openSidebar')"
        @click="drawer.open()"
      />
      <span v-else class="size-hit shrink-0" />
      <h1 class="min-w-0 flex-1 truncate text-center text-[15px] font-medium text-fg">{{ back ? '' : title }}</h1>
      <IconButton
        v-if="add && !back"
        :icon="Plus"
        variant="ghost"
        :aria-label="add"
        @click="emit('add')"
      />
      <span v-else class="size-hit shrink-0" />
    </header>
    <ScrollArea class="min-h-0 flex-1" viewport-class="px-10 pb-16 pt-8 phone:px-4 phone:pb-[calc(2rem+var(--safe-bottom))] phone:pt-2">
      <slot />
    </ScrollArea>
  </section>
</template>
