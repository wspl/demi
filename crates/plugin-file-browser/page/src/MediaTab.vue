<script setup lang="ts">
import { computed } from 'vue'
import { MediaView } from '@demicodes/plugin-sdk'
import type { MediaData } from './media'

/** The Media tab: the medium the conversation's viewer shows, whose place it keeps for a reload. */
const props = defineProps<{
  conversation: string
  tabId: string
  data: MediaData
  shown: boolean
}>()
const emit = defineEmits<{ update: [data: MediaData] }>()

const place = computed(() => (props.data.key === undefined ? undefined : { key: props.data.key, ...(props.data.row === undefined ? {} : { row: props.data.row }) }))
</script>

<template>
  <MediaView :shown="shown" :place="place" @place="emit('update', $event)" />
</template>
