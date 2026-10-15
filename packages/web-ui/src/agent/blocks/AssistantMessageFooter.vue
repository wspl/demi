<script setup lang="ts">
import { t } from '../locales/text'
import { GitFork, RefreshCw, Share } from '@lucide/vue'
import type { MessageForkState } from '../message-fork'
import type { AnswerShare } from '../sharing'
import type { VersionChoice } from '../versions'
import VersionSwitcher from './VersionSwitcher.vue'
import CopyButton from '../../ui/CopyButton.vue'
import IconButton from '../../ui/IconButton.vue'
import Tooltip from '../../ui/Tooltip.vue'
import RelativeTime from '../../ui/RelativeTime.vue'

defineProps<{
  content: string
  createdAt: string
  fork?: () => Promise<void>
  forkState?: MessageForkState
  /** No Fork at all, as on a share's page, which acts on no conversation (`sharing.md` § Safety). */
  hideFork?: boolean
  /**
   * Shares this answer, beside Fork; absent where nothing shares. It waits,
   * `shareWait`, until the message completes, as Fork does.
   */
  share?: AnswerShare
  shareWait?: boolean
  /**
   * Asks for a new answer to the message before it, offered on the last
   * answer exactly when editing that message is (`message-editing.md`
   * § Regenerate and the Up Arrow key); absent elsewhere.
   */
  regenerate?: () => void
  /** The versions of this answer, first in the footer, as ChatGPT shows them; absent with none. */
  versions?: VersionChoice
}>()
</script>

<template>
  <!-- A phone's actions are 36 px, 8 apart, each its own target. -->
  <div class="mt-2 flex flex-wrap items-center gap-1 text-fg-faint phone:gap-2">
    <VersionSwitcher v-if="versions" :choice="versions" />
    <CopyButton :source="content" />
    <Tooltip v-if="!hideFork" :content="t('transcript.assistant.fork')" :disabled="!fork">
      <IconButton
        :icon="GitFork"
        size="sm"
        variant="ghost"
        :aria-label="t('transcript.assistant.fork')"
        :disabled="!fork"
        :disabled-reason="t('transcript.assistant.forkWait')"
        :loading="forkState?.phase === 'pending'"
        @click="fork?.()"
      />
    </Tooltip>
    <Tooltip v-if="share" :content="t('transcript.assistant.share')" :disabled="shareWait">
      <IconButton
        :icon="Share"
        size="sm"
        variant="ghost"
        :aria-label="t('transcript.assistant.share')"
        :disabled="shareWait"
        :disabled-reason="t('transcript.assistant.shareWait')"
        @click="share.start()"
      />
    </Tooltip>
    <Tooltip v-if="regenerate" :content="t('transcript.assistant.regenerate')">
      <IconButton
        :icon="RefreshCw"
        size="sm"
        variant="ghost"
        :aria-label="t('transcript.assistant.regenerate')"
        @click="regenerate()"
      />
    </Tooltip>
    <RelativeTime :timestamp="createdAt" class="ml-1 text-caption leading-6 phone:leading-9" />
  </div>
</template>
