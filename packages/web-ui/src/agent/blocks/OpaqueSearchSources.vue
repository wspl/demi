<script setup lang="ts">
import type { SearchSource } from '@demicodes/protocol'
import { sourceHost, sourcePage } from '../opaque-steps'

/**
 * The pages a vendor's search returned, each a link that opens the page: its
 * title and its site, or, without a title, as a post of X comes, its
 * address. They read as thinking does, in its size and tone: what the
 * vendor found is said, not printed.
 */
defineProps<{ sources: readonly SearchSource[] }>()
</script>

<template>
  <ul class="px-3 py-1 text-conversation-secondary text-fg-subtle">
    <li v-for="source in sources" :key="source.url" class="flex min-w-0 items-baseline gap-2">
      <a
        :href="source.url"
        target="_blank"
        rel="noopener noreferrer"
        class="min-w-0 truncate hover:text-fg-body hover:underline"
      >{{ source.title ?? sourcePage(source.url) }}</a>
      <span v-if="source.title !== null" class="shrink-0 text-fg-faint">{{ sourceHost(source.url) }}</span>
    </li>
  </ul>
</template>
