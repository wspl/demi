<script setup lang="ts">
import FileIcon from '../files/FileIcon.vue'

/** A file a tool returned, as a square: its picture, or its kind's icon. */
defineProps<{
  name: string
  src?: string
}>()
</script>

<template>
  <span
    class="relative block size-12"
    :aria-label="name"
  >
    <img
      v-if="src"
      :src="src"
      :alt="name"
      class="size-12 rounded-lg object-cover ring-1 ring-line"
    />
    <span
      v-else
      class="flex size-12 items-center justify-center rounded-lg bg-surface ring-1 ring-line"
    >
      <FileIcon
        :name="name"
        :is-directory="false"
        :size="28"
      />
    </span>
  </span>
</template>
