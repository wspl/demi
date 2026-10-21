<script setup lang="ts">
import { t } from '../locales/text'
import { computed } from 'vue'
import ActivityMark from '@demicodes/web-ui/ui/ActivityMark.vue'
import type { ActivityKind } from '../activity-slot'
import { useElapsedTime } from '../../composables/useElapsedTime'
import { providerWaitLabel } from '../thinking-label'
import FunctionalBlock from './FunctionalBlock.vue'

/**
 * The transcript's tail row while it waits: a FunctionalBlock face with the
 * wait mark and why. A step or a Stop that follows the wait takes the row's
 * place and rolls over it (`RowRoll`); the answer or a failure appears in it.
 */
const props = defineProps<{
  kind: ActivityKind
  /**
   * When the server began the wait, as `Date.now()`; Requesting shows how
   * long it has waited since. Without it, as while the page's own action has
   * no answer, and for Connecting, the row counts nothing.
   */
  since?: number | null
}>()

const requestingElapsed = useElapsedTime(
  () => props.since ?? Number.NaN,
  () => props.kind === 'requesting' && props.since != null,
)

const label = computed(() => {
  switch (props.kind) {
    case 'connecting':
      return t('transcript.connecting')
    case 'requesting':
      return providerWaitLabel(requestingElapsed.value ?? 0)
  }
})
</script>

<template>
  <!--
    data-transcript-wait: the product checks tell a turn on its way by it, in
    any language (`browse.md`).
  -->
  <div class="px-[var(--agent-pad-x,2rem)]" :data-transcript-wait="kind">
    <!-- The wait mark stays while only the reason changes. -->
    <FunctionalBlock
      loading
      :roll-key="kind"
      icon-key="wait"
    >
      <template #icon>
        <ActivityMark />
      </template>
      <span class="min-w-0 truncate">{{ label }}</span>
    </FunctionalBlock>
  </div>
</template>
