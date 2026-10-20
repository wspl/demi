<script setup lang="ts">
import { t } from './locales/text'
import { t as shared } from '../locales/text'
import { ArrowUp, Paperclip, Pencil, RotateCw, Trash2 } from '@lucide/vue'
import IconButton from '../ui/IconButton.vue'
import { ICON_PX } from '../ui/icon-metrics'
import MenuItem from '../ui/MenuItem.vue'
import type { Action } from '../ui/operations'
import ScrollArea from '../ui/ScrollArea.vue'
import Tooltip from '../ui/Tooltip.vue'
import type { SavedRow } from './saved'

/**
 * The list of messages saved for later under the composer, about four rows tall
 * and scrolling within.
 *
 * A fifth row shows half, so the list reads as one that goes on.
 *
 * Each row is a menu row:
 *
 * - It shows the message's first line and, when the message has files, their
 *   count, with no time.
 * - Pressing it edits the message.
 * - Edit, Send and Delete always stand at the row's end, where the row keeps
 *   their inset, the count before them. They are quiet, and take the normal
 *   icon color while the pointer or the focus is on the row.
 *
 * A message the server did not save reads Not Saved in secondary red, with
 * Retry in Send's place.
 */
defineProps<{
  rows: SavedRow[]
  /**
   * Each action may return a promise, which its control shows until it settles.
   */
  onEdit?: Action<[id: string]>
  onSend?: Action<[id: string]>
  onDelete?: Action<[id: string]>
  onRetry?: Action<[id: string]>
}>()

/**
 * Edit, Send, Retry and Delete of one saved message exclude each other
 * (`operations.ts`).
 */
function onSaved(action: 'edit' | 'send' | 'retry' | 'delete', id: string) {
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
          <span v-if="row.state === 'notSaved'" class="mr-1 whitespace-nowrap text-on-danger">{{ t('composer.saved.notSaved') }}</span>
          <span v-if="row.attachmentCount" class="mr-1 inline-flex items-center gap-0.5 whitespace-nowrap text-fg-subtle">
            <Paperclip :size="ICON_PX.in28" class="shrink-0" />
            {{ row.attachmentCount }}
          </span>
          <Tooltip :content="t('composer.saved.edit')" class="inline-flex saved-action">
            <IconButton
              :icon="Pencil"
              variant="ghost"
              size="xs"
              :aria-label="t('composer.saved.edit')"
              :operation="onSaved('edit', row.id)"
              @click.stop="onEdit?.(row.id)"
            />
          </Tooltip>
          <Tooltip v-if="row.state === 'notSaved'" :content="shared('common.retry')" class="inline-flex saved-action">
            <IconButton
              :icon="RotateCw"
              variant="ghost"
              size="xs"
              :aria-label="shared('common.retry')"
              :operation="onSaved('retry', row.id)"
              @click.stop="onRetry?.(row.id)"
            />
          </Tooltip>
          <Tooltip v-else :content="t('composer.saved.send')" class="inline-flex saved-action">
            <IconButton
              :icon="ArrowUp"
              variant="ghost"
              size="xs"
              :aria-label="t('composer.saved.send')"
              :operation="onSaved('send', row.id)"
              @click.stop="onSend?.(row.id)"
            />
          </Tooltip>
          <Tooltip :content="t('composer.saved.delete')" class="inline-flex saved-action">
            <IconButton
              :icon="Trash2"
              variant="ghost"
              size="xs"
              :aria-label="t('composer.saved.delete')"
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
