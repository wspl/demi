<script setup lang="ts">
import { computed } from 'vue'
import { Download } from '@lucide/vue'
import Button from '../ui/Button.vue'
import TruncatedText from '../ui/TruncatedText.vue'
import { ICON_PX } from '../ui/icon-metrics'
import FileIcon from './FileIcon.vue'
import { downloadUrl } from './download'
import { formatModified } from '../ui/dates'
import { entryKind, formatBytes } from './format'
import { t } from './locales/text'
import type { FileDescription } from './types'
import type { SentenceText } from '../ui/ui-text'

/**
 * A file the view does not show: its kind, size and modification time, why
 * it is not shown when there is a reason worth saying, and Download.
 */
const props = defineProps<{
  name: string
  description: FileDescription | null
  note?: SentenceText | null
  /** The file as an attachment; absent when it cannot be downloaded. */
  download?: string | null
}>()

const facts = computed(() => [
  entryKind(props.name, false),
  props.description ? formatBytes(props.description.size) : null,
  props.description ? formatModified(props.description.modifiedAt ?? undefined) : null,
].filter((fact) => fact).join(' · '))
</script>

<template>
  <div class="flex h-full min-h-0 flex-col items-center justify-center gap-2 px-6 py-8 text-center">
    <FileIcon :name="name" :is-directory="false" :size="40" />
    <TruncatedText class="mt-1 max-w-full text-chrome text-fg" :text="name" />
    <div class="text-detail text-fg-muted">{{ facts }}</div>
    <div v-if="note" class="text-detail text-fg-muted">{{ note }}</div>
    <Button v-if="download" class="mt-2" size="sm" @click="downloadUrl(download, name)">
      <Download :size="ICON_PX.in24" />
      {{ t('actions.download') }}
    </Button>
  </div>
</template>
