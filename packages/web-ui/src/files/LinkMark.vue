<script setup lang="ts">
import { Forward } from '@lucide/vue'
import Tooltip from '../ui/Tooltip.vue'
import { linkDescription } from './format'
import type { FileBrowserLink } from './types'

/**
 * The mark at the end of a symbolic link's row, as Visual Studio Code's
 * Explorer decorates one (`file-previews.md` § Symbolic links): a small
 * arrow in the secondary text color, whose tooltip names the target as
 * written, and says when nothing is there.
 */
defineProps<{
  link: FileBrowserLink
}>()
</script>

<template>
  <Tooltip :content="linkDescription(link)" placement="bottom" class="flex size-4 shrink-0 items-center justify-center text-fg-muted">
    <Forward :size="12" :stroke-width="2" role="img" :aria-label="linkDescription(link)" />
  </Tooltip>
</template>
