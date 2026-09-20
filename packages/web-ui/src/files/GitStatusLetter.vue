<script setup lang="ts">
import { computed } from 'vue'
import Tooltip from '../ui/Tooltip.vue'
import { gitMark, type GitMarkColor } from './git-status'

/**
 * The letter VS Code's Git marks a file with, from git's two status letters
 * for it (`gitMark`), in VS Code's color for it and with VS Code's words for
 * it as the tooltip. The box is one letter wide whatever the letter, and
 * stays when VS Code does not mark the file, so the letters line up down a
 * list.
 */
const props = defineProps<{
  /** git's two status letters, as `git status --porcelain` prints them. */
  status: string
}>()

const mark = computed(() => gitMark(props.status))

/** VS Code's git decoration colors in the page's palette: its greens, its amber, its reds. */
const TONES: Record<GitMarkColor, string> = {
  added: 'text-on-success',
  renamed: 'text-on-success',
  untracked: 'text-on-success',
  modified: 'text-on-warning',
  deleted: 'text-on-danger',
  conflicting: 'text-on-danger',
}
</script>

<template>
  <Tooltip :content="mark?.text" class="flex w-3 shrink-0 justify-center">
    <span v-if="mark" class="text-[12px] font-medium" :class="TONES[mark.color]" :aria-label="mark.text">{{ mark.letter }}</span>
  </Tooltip>
</template>
