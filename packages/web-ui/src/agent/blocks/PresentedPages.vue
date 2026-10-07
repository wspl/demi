<script setup lang="ts">
import { computed } from 'vue'
import { Globe } from '@lucide/vue'
import Button from '../../ui/Button.vue'
import { ICON_PX } from '../../ui/icon-metrics'
import type { ToolCallBlock } from '../block-types'
import { storedShellView } from '../block-helpers'
import { usePageOpening } from '../page-opening'

/**
 * The pages a shell call's command presented with `demi browser present`
 * (`preview.md` § Presenting a page): a card for each, with the page's
 * title, its address, and Open, which opens it in a tab of the user's own
 * browser. Without a plugin that opens pages, the cards only name them.
 */
const props = defineProps<{ block: ToolCallBlock }>()
const pages = computed(() => storedShellView(props.block)?.presented ?? [])
const open = usePageOpening()
</script>

<template>
  <div
    v-if="pages.length > 0"
    class="flex flex-col gap-1.5 py-1 pr-3"
    :style="{ paddingLeft: `${ICON_PX.in28 + 8}px` }"
  >
    <!-- A card is 52px high: 8px above and below two lines of 20px and 16px. Open, 24px high, keeps
         14px to the card's right edge as to its top and bottom. -->
    <div
      v-for="page in pages"
      :key="page.tab"
      class="flex min-w-0 max-w-120 items-center gap-2.5 rounded-md border border-line-subtle bg-surface-base py-2 pr-3.5 pl-3"
    >
      <Globe :size="ICON_PX.in28" class="shrink-0 text-fg-subtle" />
      <div class="flex min-w-0 flex-1 flex-col">
        <span class="truncate leading-5 text-fg-emphasis">{{ page.title || page.url }}</span>
        <span class="truncate text-[12px] leading-4 text-fg-muted">{{ page.url }}</span>
      </div>
      <Button v-if="open()" size="sm" @click="open()?.(page)">Open</Button>
    </div>
  </div>
</template>
