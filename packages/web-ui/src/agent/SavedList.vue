<script setup lang="ts">
import { ArrowUp, Paperclip, Pencil, Trash2 } from '@lucide/vue'
import IconButton from '../ui/IconButton.vue'
import { ICON_PX } from '../ui/icon-metrics'
import MenuItem from '../ui/MenuItem.vue'
import type { Action } from '../ui/operations'
import ScrollArea from '../ui/ScrollArea.vue'
import Tooltip from '../ui/Tooltip.vue'
import type { SavedRow } from './saved'

/**
 * The list of messages saved for later under the composer
 * (`product.md` § Saved for later), about four
 * rows tall and scrolling within; a fifth row shows half, so the list reads
 * as one that goes on. Each row is a menu row: its first line and, when the
 * message has files, their count, with no time; pressing it edits the
 * message.
 * Edit, Send and Delete always stand at the row's end, where the row keeps
 * their inset, the count before them: quiet, and the normal icon color while
 * the pointer or the focus is on the row.
 */
defineProps<{
  rows: SavedRow[]
  /** Each action may return a promise, which its control shows until it settles. */
  onEdit?: Action<[id: string]>
  onSend?: Action<[id: string]>
  onDelete?: Action<[id: string]>
}>()

/** Edit, Send and Delete of one saved message exclude each other (`operations.ts`). */
function onSaved(action: 'edit' | 'send' | 'delete', id: string) {
  return { action, target: `saved:${id}`, group: 'saved' }
}
</script>

<template>
  <div class="saved-list flex flex-col rounded-lg bg-surface-raised outline outline-1 outline-line">
    <ScrollArea class="flex-auto" viewport-class="overlay-menu-scroll p-1">
      <MenuItem
        v-for="row in rows"
        :key="row.id"
        iconless
        :operation="onSaved('edit', row.id)"
        @select="onEdit?.(row.id)"
      >
        <span class="min-w-0 truncate">{{ row.preview }}</span>
        <template #actions>
          <span v-if="row.attachmentCount" class="mr-1 inline-flex items-center gap-0.5 whitespace-nowrap text-fg-subtle">
            <Paperclip :size="ICON_PX.in28" class="shrink-0" />
            {{ row.attachmentCount }}
          </span>
          <Tooltip content="Edit" class="inline-flex saved-action">
            <IconButton
              :icon="Pencil"
              variant="ghost"
              size="xs"
              aria-label="Edit"
              :operation="onSaved('edit', row.id)"
              @click.stop="onEdit?.(row.id)"
            />
          </Tooltip>
          <Tooltip content="Send" class="inline-flex saved-action">
            <IconButton
              :icon="ArrowUp"
              variant="ghost"
              size="xs"
              aria-label="Send"
              :operation="onSaved('send', row.id)"
              @click.stop="onSend?.(row.id)"
            />
          </Tooltip>
          <Tooltip content="Delete" class="inline-flex saved-action">
            <IconButton
              :icon="Trash2"
              variant="ghost"
              size="xs"
              aria-label="Delete"
              :operation="onSaved('delete', row.id)"
              @click.stop="onDelete?.(row.id)"
            />
          </Tooltip>
        </template>
      </MenuItem>
    </ScrollArea>
  </div>
</template>

<style scoped>
.saved-list :deep(.saved-action [role='button']) {
  color: var(--color-fg-subtle);
}

.saved-list :deep(.menu-row:hover .saved-action [role='button']),
.saved-list :deep(.menu-row:focus-within .saved-action [role='button']) {
  color: var(--color-fg-muted);
}

.saved-list :deep(.menu-row:hover .saved-action [role='button']:hover),
.saved-list :deep(.menu-row:focus-within .saved-action [role='button']:hover) {
  color: var(--color-fg-body);
}

.saved-list {
  /* Four rows and a half, with their 1px gaps and the list's padding. */
  max-height: calc(4.5 * 1.75rem + 4px + 0.5rem);
}
</style>
