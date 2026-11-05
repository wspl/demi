<script setup lang="ts">
import { t } from './locales/text'
import { inject, onBeforeUnmount, watch } from 'vue'
import { ChevronLeft } from '@lucide/vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import ScrollArea from '@demicodes/web-ui/ui/ScrollArea.vue'
import TruncatedText from '@demicodes/web-ui/ui/TruncatedText.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import type { TitleText } from '../ui/ui-text'
import { settingsLevelKey, settingsPageKey, type SettingsLevel } from './navigation'

/**
 * A list beside the thing it selects, inside one settings page.
 *
 * - Wide hosts show both. Narrow ones show the list, then the detail as a page of
 *   its own.
 * - In the settings dialog the detail takes over the dialog's one navigation bar,
 *   whose back button names the page it returns to, and the page's title steps
 *   aside. Elsewhere the split draws its own back row.
 * - A list of many entries, each named and described in two lines, takes a wider
 *   rail (`wideList`), and its search and its buttons stay above the entries
 *   (`listHeader`) while they scroll.
 * - The list has no surface of its own, so it reads as a rail beside the page,
 *   not a nested page.
 */
defineProps<{
  /** Title of the open detail, for the narrow back row. */
  detailTitle?: TitleText
  /** The rail of a list of many two-line entries, such as credentials. */
  wideList?: boolean
}>()

const detailOpen = defineModel<boolean>('detailOpen', { default: false })

const bar = inject(settingsLevelKey, null)
const page = inject(settingsPageKey, null)
/** The level this split put on the dialog's bar, which only it takes off again. */
let level: SettingsLevel | null = null

function leaveBar(): void {
  if (bar && level && bar.value === level) {
    bar.value = null
  }
  level = null
  if (page) {
    page.nested.value = false
  }
}

watch(
  detailOpen,
  (open) => {
    if (!bar || !open) {
      leaveBar()
      return
    }
    level = {
      label: page?.title() ?? t('dialog.back'),
      back: () => {
        detailOpen.value = false
      },
    }
    bar.value = level
    if (page) {
      page.nested.value = true
    }
  },
  { immediate: true },
)
onBeforeUnmount(leaveBar)
</script>

<template>
  <div class="@container min-h-0 flex-1">
    <!-- The list is a bare rail, not a card: only the detail's own groups draw surfaces.
         Each side scrolls on its own when the host gives the split a height. -->
    <div
      class="grid h-full grid-rows-[minmax(0,1fr)] gap-x-8 gap-y-4"
      :class="wideList ? 'wide:grid-cols-[15rem_minmax(0,1fr)]' : 'wide:grid-cols-[13rem_minmax(0,1fr)]'"
    >
      <div class="min-h-0 flex-col" :class="detailOpen ? 'hidden wide:flex' : 'flex'">
        <div v-if="$slots.listHeader" class="shrink-0 pb-2"><slot name="listHeader" /></div>
        <!-- Scroll regions clip both axes; the -mx/px pair leaves room for rings at the edges.
             The entries stack in a grid, not a flex column: a column would shrink each entry
             below its height once the list outgrows the rail, so the rows would close up. -->
        <ScrollArea class="-mx-1 flex min-h-0 flex-1" viewport-class="grid grid-cols-[minmax(0,1fr)] content-start gap-0.5 px-1">
          <slot name="list" />
        </ScrollArea>
      </div>
      <!-- The detail's region reaches into the page's right margin, so its scrollbar runs
           there, as the page's own does, and never over the detail's cards. -->
      <ScrollArea
        class="-ml-1 -mr-5 min-h-0 min-w-0"
        :class="detailOpen ? 'flex' : 'hidden wide:flex'"
        viewport-class="pl-1 pr-5"
      >
        <div v-if="!bar" class="-mx-2 mb-3 flex h-10 items-center gap-1 wide:hidden">
          <Button
            variant="ghost"
            size="sm"
            @click="detailOpen = false"
          >
            <ChevronLeft :size="ICON_PX.in24" />
            {{ t('dialog.back') }}
          </Button>
          <TruncatedText class="text-chrome text-fg" :text="detailTitle ?? ''" />
        </div>
        <slot name="detail" />
      </ScrollArea>
    </div>
  </div>
</template>
