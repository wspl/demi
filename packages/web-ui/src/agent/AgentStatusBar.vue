<script setup lang="ts">
import AgentRunLine from './AgentRunLine.vue'
import type { AgentRun } from './subagents'

/**
 * The thin bar along the Agents window's bottom edge that says what the
 * shown child is doing and runs on (`subagents.md` § What an agent runs on),
 * in the secondary text color under its own top divider. Its side padding is
 * the distance the profile tag keeps to the bar's top and bottom,
 * (32px - 18px) / 2, as every control at a container's edge does; on a phone the bar is 36px tall and its text 15px, as a tab's.
 */
defineProps<{ run: AgentRun }>()
</script>

<template>
  <div class="flex h-8 phone:h-9 shrink-0 items-center border-t border-line px-[7px] phone:px-[9px] text-chrome phone:text-[15px] text-fg-muted">
    <AgentRunLine class="min-w-0 flex-1" :run="run" />
  </div>
</template>
