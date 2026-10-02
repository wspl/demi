<script setup lang="ts">
import { PageScope, type AnyPluginPage } from './page'

/**
 * The conversation header's tools of the plugins the user has on, each of
 * which shows itself only while its plugin has something to show.
 */
defineProps<{
  pages: readonly AnyPluginPage[]
  enabled: (plugin: string) => boolean
  conversation: string
}>()
</script>

<template>
  <template v-for="page in pages" :key="page.plugin">
    <PageScope
      v-if="page.headerTool && enabled(page.plugin)"
      :page="page"
      :component="page.headerTool"
      :props="{ conversation }"
    />
  </template>
</template>
