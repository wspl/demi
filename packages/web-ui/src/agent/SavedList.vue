<script setup lang="ts">
import { t } from './locales/text'
import { ArrowUp, Pencil, Trash2 } from '@lucide/vue'
import IconButton from '../ui/IconButton.vue'
import MenuItem from '../ui/MenuItem.vue'
import type { Action } from '../ui/operations'
import ScrollArea from '../ui/ScrollArea.vue'
import Tooltip from '../ui/Tooltip.vue'
import AttachmentCapsule from './message-editor/AttachmentCapsule.vue'
import type { SavedRow } from './saved'

/**
 * The list of messages saved for later under the composer, at most half the
 * window tall and scrolling within, so a long list is found by scrolling
 * little.
 *
 * Each row is a menu row:
 *
 * - It shows the message on one line, each file as its capsule where it
 *   stands, as the composer shows it, with no time.
 * - Pressing it edits the message.
 * - A message the server has not taken yet reads Saving… before the actions.
 * - Edit, Send and Delete always stand at the row's end, where the row keeps
 *   their inset. They are quiet, and take the normal icon color while the
 *   pointer or the focus is on the row.
 */
defineProps<{
  rows: SavedRow[]
  /**
   * Each action may return a promise, which its control shows until it settles.
   */
  onEdit?: Action<[id: string]>
  onSend?: Action<[id: string]>
  onDelete?: Action<[id: string]>
}>()

/**
 * Edit, Send and Delete of one saved message exclude each other
 * (`operations.ts`).
 */
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
        <span class="min-w-0 truncate">
          <template v-for="(part, index) in row.parts" :key="index">
            <AttachmentCapsule v-if="'capsule' in part" :capsule="part.capsule" />
            <template v-else>{{ part.text }}</template>
          </template>
        </span>
        <template #actions>
          <span v-if="row.state === 'saving'" class="mr-1 whitespace-nowrap text-fg-subtle">{{ t('composer.saved.saving') }}</span>
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
          <Tooltip :content="t('composer.saved.send')" class="inline-flex saved-action">
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
  max-height: 50dvh;
}
</style>
