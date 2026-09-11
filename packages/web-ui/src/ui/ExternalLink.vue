<script setup lang="ts">
import { ExternalLink as ExternalLinkIcon } from '@lucide/vue'
import { ICON_PX } from './icon-metrics'

/**
 * A link that leaves the app for another site. It reads as a link (muted,
 * underlined on hover) and says so with the outward arrow, so it is never
 * mistaken for a caption. Opens in a new tab without a referrer.
 */
defineProps<{
  href: string
}>()
</script>

<template>
  <a
    :href="href"
    target="_blank"
    rel="noopener noreferrer"
    class="inline-flex select-none items-center gap-1 text-[12px] text-fg-muted underline-offset-2 transition-colors duration-200 ease-out hover:text-fg-body hover:underline"
  >
    <slot />
    <ExternalLinkIcon :size="ICON_PX.in20" class="shrink-0" aria-hidden="true" />
  </a>
</template>
