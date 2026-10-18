<script setup lang="ts">
import { ref, watch } from 'vue'
import { Check, Pencil, Trash2 } from '@lucide/vue'
import IconButton from '../../ui/IconButton.vue'
import Tooltip from '../../ui/Tooltip.vue'
import { ICON_PX } from '../../ui/icon-metrics'
import { t } from '../locales/text'
import { t as shared } from '../../locales/text'
import CommentBox from './CommentBox.vue'

/**
 * A comment as its card and the composer's list show it: its text, read as
 * text, with ghost Delete and Edit at the row's right end; Edit turns the
 * row into the editor. A sent comment stays as it was sent and says so.
 */
const props = defineProps<{
  text: string
  sent: boolean
  /** The text it is about, shown above it where the list stands away from the reply. */
  quote?: string
}>()

const emit = defineEmits<{
  save: [text: string]
  delete: []
  /** The row became the editor, or left it; its card stays open meanwhile. */
  editing: [editing: boolean]
}>()

const editing = ref(false)
watch(editing, (now) => emit('editing', now))

function save(text: string): void {
  editing.value = false
  emit('save', text)
}
</script>

<template>
  <div class="flex flex-col gap-1.5" data-comment>
    <p v-if="quote" class="line-clamp-2 border-l-2 border-[var(--comment-ink)] pl-2 text-detail text-fg-muted">{{ quote }}</p>
    <CommentBox v-if="editing" :text="text" @save="save" @cancel="editing = false" />
    <div v-else class="flex items-start gap-1">
      <p class="min-w-0 flex-1 whitespace-pre-wrap py-1 text-explanation text-fg-body">{{ text }}</p>
      <span v-if="props.sent" class="inline-flex h-7 shrink-0 items-center gap-1 text-detail text-fg-subtle">
        <Check :size="ICON_PX.in24" aria-hidden="true" />
        {{ t('comments.sent') }}
      </span>
      <template v-else>
        <Tooltip :content="shared('common.delete')">
          <IconButton :icon="Trash2" size="sm" variant="ghost" :aria-label="shared('common.delete')" @click="emit('delete')" />
        </Tooltip>
        <Tooltip :content="t('transcript.user.edit')">
          <IconButton :icon="Pencil" size="sm" variant="ghost" :aria-label="t('transcript.user.edit')" @click="editing = true" />
        </Tooltip>
      </template>
    </div>
  </div>
</template>
