<script setup lang="ts">
import { ChevronLeft } from '@lucide/vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'

/**
 * A list beside the thing it selects, inside one settings page. Wide hosts show
 * both; narrow ones show the list, then the detail behind a back button. The list
 * has no surface of its own, so it reads as a rail beside the page, not a nested page.
 */
defineProps<{
  /** Title of the open detail, for the narrow back row. */
  detailTitle?: string
}>()

const detailOpen = defineModel<boolean>('detailOpen', { default: false })
</script>

<template>
  <div class="@container min-h-0 flex-1">
    <!-- The list is a bare rail, not a card: only the detail's own groups draw surfaces.
         Each side scrolls on its own when the host gives the split a height. -->
    <div class="grid h-full grid-rows-[minmax(0,1fr)] gap-x-8 gap-y-4 @md:grid-cols-[11rem_minmax(0,1fr)]">
      <!-- Scroll regions clip both axes; the -mx/px pair leaves room for rings at the edges. -->
      <aside
        class="-mx-1 flex min-h-0 flex-col gap-0.5 overflow-y-auto px-1"
        :class="detailOpen ? 'hidden @md:flex' : 'flex'"
      >
        <slot name="list" />
      </aside>
      <section class="-mx-1 min-h-0 min-w-0 overflow-y-auto px-1" :class="detailOpen ? 'block' : 'hidden @md:block'">
        <div class="-mx-2 mb-3 flex h-10 items-center gap-1 @md:hidden">
          <Button variant="ghost" size="sm" @click="detailOpen = false">
            <ChevronLeft :size="ICON_PX.in24" />
            Back
          </Button>
          <span class="min-w-0 truncate text-chrome text-fg">{{ detailTitle }}</span>
        </div>
        <slot name="detail" />
      </section>
    </div>
  </div>
</template>
