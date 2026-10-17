<script setup lang="ts">
import { computed } from 'vue'
import { answerActions, type AnswerActionsSource } from '../answer-actions'
import type { VersionChoice } from '../versions'
import VersionSwitcher from './VersionSwitcher.vue'
import CopyButton from '../../ui/CopyButton.vue'
import IconButton from '../../ui/IconButton.vue'
import Tooltip from '../../ui/Tooltip.vue'
import RelativeTime from '../../ui/RelativeTime.vue'

/**
 * An answer's footer: its versions, then its actions as buttons, which
 * `answerActions` owns with the answer's menu, then when it was written.
 */
const props = defineProps<AnswerActionsSource & {
  content: string
  createdAt: string
  /** The versions of this answer, first in the footer, as ChatGPT shows them; absent with none. */
  versions?: VersionChoice
}>()

// Copy is the one Copy control, with its own Copied (CopyButton): its action here only holds its place.
const actions = computed(() => answerActions(props, () => undefined))
</script>

<template>
  <!-- A phone's actions are 36 px, 8 apart, each its own target. -->
  <div class="mt-2 flex flex-wrap items-center gap-1 text-fg-faint phone:gap-2">
    <VersionSwitcher v-if="versions" :choice="versions" />
    <template v-for="action in actions" :key="action.key">
      <CopyButton v-if="action.key === 'copy'" :source="content" />
      <Tooltip v-else :content="action.label" :disabled="action.disabled">
        <IconButton
          :icon="action.icon"
          size="sm"
          variant="ghost"
          :aria-label="action.label"
          :disabled="action.disabled"
          :disabled-reason="action.disabledReason"
          :loading="action.loading"
          @click="action.run()"
        />
      </Tooltip>
    </template>
    <RelativeTime :timestamp="createdAt" class="ml-1 text-caption leading-6 phone:leading-9" />
  </div>
</template>
