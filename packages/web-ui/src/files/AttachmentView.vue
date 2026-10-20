<script setup lang="ts">
import { computed } from 'vue'
import FileContent from './FileContent.vue'
import { attachmentPath, attachmentSource } from './attachment-source'
import { formatBytes } from './format'

/**
 * An attachment previewed in the work panel as the File view previews a
 * file, from the attachment's stored copy: the same previews under the same
 * bar, which names the file and its size and offers Download. The host keeps
 * the Preview or Source choice (v-model), as the File view's does.
 */
const props = defineProps<{
  name: string
  /** In bytes. */
  size: number
  /** Where its stored copy loads from. */
  url: string
}>()

const mode = defineModel<'preview' | 'source'>('mode', { default: 'preview' })

const source = computed(() => attachmentSource({ name: props.name, size: props.size, url: props.url }))
const label = computed(() => `${props.name} · ${formatBytes(props.size)}`)
</script>

<template>
  <!-- A link in a document names a file of no Host, so it opens nothing. -->
  <FileContent
    v-model:mode="mode"
    :source="source"
    root="/"
    :path="attachmentPath({ name })"
    :label="label"
  />
</template>
