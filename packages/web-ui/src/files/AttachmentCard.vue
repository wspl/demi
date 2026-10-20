<script setup lang="ts">
import { computed } from 'vue'
import { Download } from '@lucide/vue'
import IconButton from '../ui/IconButton.vue'
import Tooltip from '../ui/Tooltip.vue'
import TruncatedText from '../ui/TruncatedText.vue'
import FileIcon from './FileIcon.vue'
import { downloadUrl } from './download'
import { fileKindName } from './file-bar'
import { formatBytes } from './format'
import { t } from './locales/text'

/**
 * An attachment block's card: the file's icon, its name, its kind and size, and
 * Download at the right end. A click on the card asks to open the file (`open`)
 * where the page can open it (`openable`); only Download downloads. While the
 * page still asks for the attachment (`file` null), the card keeps its place,
 * empty.
 */
const props = defineProps<{
  file: { name: string; size: number; url: string } | null
  openable: boolean
}>()
const emit = defineEmits<{ open: [] }>()

const facts = computed(() => props.file ? `${fileKindName(props.file.name)} · ${formatBytes(props.file.size)}` : '')
const opens = computed(() => props.file !== null && props.openable)

function open(): void {
  if (opens.value)
    emit('open')
}

/** Return or Space on the card itself, not on its Download, opens it as a button does. */
function press(event: KeyboardEvent): void {
  if (event.target !== event.currentTarget || (event.key !== 'Enter' && event.key !== ' '))
    return
  event.preventDefault()
  open()
}

function download(): void {
  if (props.file)
    downloadUrl(props.file.url, props.file.name)
}
</script>

<template>
  <div
    class="flex h-14 w-[360px] max-w-full select-none items-center gap-3 rounded-lg border border-line bg-surface pl-3 pr-2 transition-colors duration-200 ease-out"
    :class="opens ? 'cursor-default hover:bg-hover focus-visible:bg-hover focus-visible:outline-none' : ''"
    :role="opens ? 'button' : undefined"
    :tabindex="opens ? 0 : undefined"
    :aria-label="opens ? file?.name : undefined"
    data-attachment-card-body
    @click="open"
    @keydown="press"
  >
    <template v-if="file">
      <FileIcon :name="file.name" :is-directory="false" :size="32" />
      <div class="min-w-0 flex-1">
        <TruncatedText class="text-chrome text-fg" :text="file.name" />
        <div class="truncate text-detail tabular-nums text-fg-muted">{{ facts }}</div>
      </div>
      <Tooltip :content="t('actions.download')">
        <IconButton :icon="Download" variant="ghost" :aria-label="t('actions.download')" @click.stop="download" />
      </Tooltip>
    </template>
  </div>
</template>
