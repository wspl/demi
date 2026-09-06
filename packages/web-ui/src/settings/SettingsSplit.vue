<script setup lang="ts">
import { ChevronLeft } from '@lucide/vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'

/**
 * A list beside the thing it selects, inside one settings page. Wide hosts show
 * both; narrow ones show the list, then the detail behind a back button.
 */
defineProps<{
  /** Title of the open detail, for the narrow back row. */
  detailTitle?: string
}>()

const detailOpen = defineModel<boolean>('detailOpen', { default: false })
</script>

<template>
  <div class="@container">
    <div class="grid overflow-hidden rounded-xl border border-line bg-surface-float @md:grid-cols-[12rem_minmax(0,1fr)]">
      <aside
        class="flex flex-col gap-0.5 border-line bg-surface/60 p-2 @md:border-r"
        :class="detailOpen ? 'hidden @md:flex' : 'flex'"
      >
        <slot name="list" />
      </aside>
      <section class="min-w-0" :class="detailOpen ? 'block' : 'hidden @md:block'">
        <div class="flex h-10 items-center gap-1 border-b border-line-subtle px-2 @md:hidden">
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
