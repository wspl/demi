<script setup lang="ts">
/**
 * A linked worktree's glyph: VS Code's worktree codicon, the one its Source
 * Control view marks a worktree with, read from the package so its license
 * reaches the notices. Lucide, the other glyphs' set, has none: its branch,
 * fork and folder glyphs already mean other things here.
 *
 * It takes a Lucide glyph's `size` and paints in `currentColor`, filled, as
 * codicons are drawn.
 */
import worktree from '@vscode/codicons/src/icons/worktree.svg?raw'

defineProps<{ size: number }>()

const path = /\sd="([^"]+)"/.exec(worktree)![1]
</script>

<template>
  <svg :width="size" :height="size" viewBox="0 0 16 16" fill="currentColor" class="shrink-0">
    <path :d="path" />
  </svg>
</template>
