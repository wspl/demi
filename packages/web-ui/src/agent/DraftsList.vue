<script setup lang="ts">
import { ArrowUp, Paperclip, Pencil, Trash2 } from '@lucide/vue'
import IconButton from '../ui/IconButton.vue'
import { ICON_PX } from '../ui/icon-metrics'
import MenuItem from '../ui/MenuItem.vue'
import RelativeTime from '../ui/RelativeTime.vue'
import ScrollArea from '../ui/ScrollArea.vue'
import Tooltip from '../ui/Tooltip.vue'
import type { DraftRow } from './drafts'

/**
 * The list of drafts under the composer (`product.md` § Drafts), about four
 * rows tall and scrolling within; a fifth row shows half, so the list reads
 * as one that goes on. Each row is a menu row: its first line, its files'
 * count and when it was saved; pressing it edits the draft, and while the
 * pointer is on it, Edit, Send and Delete stand in place of the count and
 * the time, at the row's end, where the row keeps their inset.
 */
defineProps<{ rows: DraftRow[] }>()

const emit = defineEmits<{
  edit: [id: string]
  send: [id: string]
  delete: [id: string]
}>()
</script>

<template>
  <div class="drafts-list flex flex-col rounded-lg bg-surface-raised outline outline-1 outline-line">
    <ScrollArea class="flex-auto" viewport-class="overlay-menu-scroll p-1">
      <MenuItem
        v-for="row in rows"
        :key="row.id"
        iconless
        reveal-actions
        @select="emit('edit', row.id)"
      >
        <span class="min-w-0 truncate">{{ row.preview }}</span>
        <template #suffix>
          <span class="flex items-center gap-2 whitespace-nowrap text-fg-subtle">
            <span v-if="row.attachmentCount" class="inline-flex items-center gap-0.5">
              <Paperclip :size="ICON_PX.in28" class="shrink-0" />
              {{ row.attachmentCount }}
            </span>
            <RelativeTime :timestamp="row.savedAt" />
          </span>
        </template>
        <template #actions>
          <Tooltip content="Edit" class="inline-flex">
            <IconButton
              :icon="Pencil"
              variant="ghost"
              size="xs"
              aria-label="Edit"
              @click.stop="emit('edit', row.id)"
            />
          </Tooltip>
          <Tooltip content="Send" class="inline-flex">
            <IconButton
              :icon="ArrowUp"
              variant="ghost"
              size="xs"
              aria-label="Send"
              @click.stop="emit('send', row.id)"
            />
          </Tooltip>
          <Tooltip content="Delete" class="inline-flex">
            <IconButton
              :icon="Trash2"
              variant="ghost"
              size="xs"
              aria-label="Delete"
              @click.stop="emit('delete', row.id)"
            />
          </Tooltip>
        </template>
      </MenuItem>
    </ScrollArea>
  </div>
</template>

<style scoped>
.drafts-list {
  /* Four rows and a half, with their 1px gaps and the list's padding. */
  max-height: calc(4.5 * 1.75rem + 4px + 0.5rem);
}
</style>
