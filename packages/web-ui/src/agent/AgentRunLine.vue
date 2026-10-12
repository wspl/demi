<script setup lang="ts">
import { computed } from 'vue'
import { formatSpan } from '@demicodes/utils'
import { useElapsedTime } from '../composables/useElapsedTime'
import StatusDot from '../ui/StatusDot.vue'
import Tag from '../ui/Tag.vue'
import Tooltip from '../ui/Tooltip.vue'
import AgentLink from './AgentLink.vue'
import { agentPhaseWords } from './agent-links'
import { statusTone } from './conversation-status'
import { runEffortWords, subagentStatus, type AgentRun } from './subagents'

/**
 * What a child is doing and what it runs on, in two groups
 * (`subagents.md` § What an agent runs on): its state dot, state and time on
 * the left, with *Forked from agent N* for a fork; its profile, model,
 * effort and *Fast* on the right. The time follows the page's clock while the
 * round runs. Where the room runs out, *Forked from* gives way first, then
 * the model's name, by the width of the line's own box; the state and the
 * time always stay. The caller gives its color and size.
 *
 * A `plain` line is the one in a tip, which has no box of its own to measure
 * and nothing to point at or click: it shows every part, and its fork is
 * text, not a link.
 */
const props = defineProps<{
  run: AgentRun
  plain?: boolean
}>()

const elapsedMs = useElapsedTime(
  () => Date.parse(props.run.startedAt),
  () => props.run.phase === 'running',
  () => (props.run.endedAt ? Date.parse(props.run.endedAt) : null),
)
const time = computed(() => formatSpan(elapsedMs.value ?? 0))
// The dot of the agent's tab, drawn in the flow; a Stop, like a child that ran nothing, has none.
const tone = computed(() => statusTone(subagentStatus(props.run.phase)))
const effort = computed(() => runEffortWords(props.run.model))
// The model's name gives way when the line's box is narrower than 24rem; a tip shows it always.
// *Forked from* gives way first, below 36rem.
const forkRoom = computed(() => (props.plain ? '' : 'hidden @min-[36rem]:inline'))
const modelRoom = computed(() => (props.plain ? '' : 'hidden @min-[24rem]:inline'))
</script>

<template>
  <div :class="plain ? '' : '@container min-w-0'">
    <div class="flex min-w-0 items-center gap-3 whitespace-nowrap">
      <span class="flex shrink-0 items-center gap-1.5">
        <StatusDot v-if="tone" :tone="tone" :pulse="run.phase === 'running'" />
        <span>{{ agentPhaseWords(run.phase) }}</span>
        <span class="text-fg-faint" aria-hidden="true">·</span>
        <span class="tabular-nums">{{ time }}</span>
        <!-- The fork is the same kind of fact as the time, so the same separator leads it, and goes with it. -->
        <template v-if="run.forkedFrom !== null">
          <span class="text-fg-faint" :class="forkRoom" aria-hidden="true">·</span>
          <span :class="forkRoom">
            Forked from <template v-if="plain">agent {{ run.forkedFrom }}</template><AgentLink v-else :number="run.forkedFrom" />
          </span>
        </template>
      </span>
      <span class="ml-auto flex min-w-0 items-center gap-1.5 overflow-hidden">
        <Tag v-if="run.profile" class="shrink-0">{{ run.profile }}</Tag>
        <Tooltip v-if="!plain" tag="span" :class="modelRoom" wide>
          <span>{{ run.model.model }}</span>
          <template #overlay>
            <div>{{ run.model.provider }} · {{ run.model.model }}</div>
            <div v-if="!run.profile" class="text-fg-muted">Inherited from this conversation</div>
          </template>
        </Tooltip>
        <span v-else>{{ run.model.model }}</span>
        <!-- A separator belongs to the model's name while it shows, so none stands where the name gave way. -->
        <template v-if="effort">
          <span class="text-fg-faint" :class="modelRoom" aria-hidden="true">·</span>
          <span>{{ effort }}</span>
        </template>
        <template v-if="run.model.fast">
          <span class="text-fg-faint" :class="effort ? '' : modelRoom" aria-hidden="true">·</span>
          <span>Fast</span>
        </template>
      </span>
    </div>
  </div>
</template>
