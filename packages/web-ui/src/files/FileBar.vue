<script setup lang="ts">
import { Code, Download, Eye } from '@lucide/vue'
import { computed } from 'vue'
import CopyButton from '../ui/CopyButton.vue'
import IconButton from '../ui/IconButton.vue'
import Segmented, { type SegmentedOption } from '../ui/Segmented.vue'
import TruncatedText from '../ui/TruncatedText.vue'
import Tooltip from '../ui/Tooltip.vue'
import type { FileBar } from './file-bar'
import { t } from './locales/text'

/**
 * The bar over the file the File view shows, as GitHub's over a file's
 * contents (`file-previews.md` § The File view's bars): on its left what the
 * file is, on its right what can be done with it, Preview and Source, Copy
 * of its whole text, which turns to a check for a moment, and Download.
 */
const props = defineProps<{
  bar: FileBar
  /** Preview and Source show, but the file is too large to render. */
  tooLargeToRender?: boolean
  /** The width at the bar's end that a tree shown over the view covers: the controls stay beside it. */
  overlap: number
}>()
const emit = defineEmits<{ download: [] }>()

/** Markdown and SVG: their rendered view, or their text. */
const mode = defineModel<'preview' | 'source'>('mode', { required: true })

const modeOptions = computed<readonly SegmentedOption<'preview' | 'source'>[]>(() => [
  { value: 'preview', label: t('view.preview'), icon: Eye },
  { value: 'source', label: t('view.source'), icon: Code },
])

/** The bar's height less a control's, halved: each control at the end keeps that distance to every edge. */
const END_INSET = (36 - 24) / 2
</script>

<template>
  <div
    class="flex h-9 shrink-0 select-none items-center gap-2 border-b border-line pl-3 text-[12px] text-fg-muted"
    :style="{ paddingInlineEnd: `${props.overlap + END_INSET}px` }"
  >
    <TruncatedText class="flex-1 tabular-nums" :text="bar.label" />
    <div class="flex shrink-0 items-center gap-1">
      <Segmented
        v-if="bar.modes"
        v-model="mode"
        :options="modeOptions"
        size="sm"
        :disabled="tooLargeToRender"
        :disabled-reason="t('view.tooLargeToRender')"
      />
      <CopyButton v-if="bar.copy !== null" :source="bar.copy" />
      <Tooltip v-if="bar.download" :content="t('actions.download')">
        <IconButton :icon="Download" variant="ghost" size="sm" :aria-label="t('actions.download')" @click="emit('download')" />
      </Tooltip>
    </div>
  </div>
</template>
