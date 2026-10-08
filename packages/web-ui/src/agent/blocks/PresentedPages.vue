<script setup lang="ts">
import { computed } from 'vue'
import { Globe } from '@lucide/vue'
import Button from '../../ui/Button.vue'
import ListCard from '../../ui/ListCard.vue'
import ListCardRow from '../../ui/ListCardRow.vue'
import { ICON_PX } from '../../ui/icon-metrics'
import type { ToolCallBlock } from '../block-types'
import { storedShellView } from '../block-helpers'
import { usePageOpening } from '../page-opening'

/**
 * The pages a shell call's command presented with `demi browser present`
 * (`preview.md` § Presenting a page): one row each in a list, with the
 * page's title, its address, and Open, which opens it in a tab of the
 * user's own browser. Without a plugin that opens pages, the rows only
 * name them.
 */
const props = defineProps<{ block: ToolCallBlock }>()
const pages = computed(() => storedShellView(props.block)?.presented ?? [])
const open = usePageOpening()
</script>

<template>
  <!-- The list starts where the tool row's text starts, under the row it belongs to. -->
  <div
    v-if="pages.length > 0"
    class="py-1 pr-3"
    :style="{ paddingLeft: `${ICON_PX.in28 + 8}px` }"
  >
    <ListCard class="max-w-120">
      <ListCardRow
        v-for="page in pages"
        :key="page.tab"
        :title="page.title || page.url"
        :detail="page.url"
      >
        <template #icon>
          <Globe :size="ICON_PX.in28" />
        </template>
        <template v-if="open()" #action>
          <Button size="sm" variant="ghost" @click="open()?.(page)">Open</Button>
        </template>
      </ListCardRow>
    </ListCard>
  </div>
</template>
