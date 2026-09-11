<script setup lang="ts">
import { reactive } from 'vue'
import ChatSession, {
  type ChatSessionState,
} from '@demicodes/web-ui/agent/ChatSession.vue'
import SessionStatus from '@demicodes/web-ui/agent/SessionStatus.vue'
import type { Block } from '@demicodes/core'
import { generationErrorBlock, shortTranscriptBlocks } from '../fixtures/blocks'
import GalleryComposer from './GalleryComposer.vue'
import GallerySection from './GallerySection.vue'
import GallerySpecimen from './GallerySpecimen.vue'

/**
 * Every session-level failure, pinned. Each specimen is the product's
 * ChatSession over a fixed state, so what the reader sees here is what the
 * product shows for that state; nothing here toggles.
 *
 * The rule the specimens demonstrate: a failure is named once, in flow.
 * - No history to keep: the status pane replaces the transcript (Retry lives there).
 * - History in memory: the transcript stays; an ErrorNotice at its tail names the failure (Retry lives there).
 * - The failure is a transcript record: the record itself carries Retry; the dock adds nothing.
 */
interface SessionCase {
  variant: string
  note: string
  session: ChatSessionState
  composer: 'default' | 'none' | 'noModels' | 'archived'
}

function state(
  id: string,
  partial: Partial<ChatSessionState> & { blocks?: Block[] },
): ChatSessionState {
  return reactive<ChatSessionState>({
    id: `error-${id}`,
    title: 'Build a minesweeper game',
    blocks: [],
    queue: [],
    pendingSteers: [],
    phase: 'idle',
    load: 'ready',
    lastError: null,
    archived: false,
    status: 'idle',
    scroll: null,
    subagents: [],
    terminals: [],
    ...partial,
  })
}

const SOCKET_ERROR = 'Agent socket failed to connect: ECONNREFUSED 127.0.0.1:18911'

const cases: SessionCase[] = [
  {
    variant: 'Initial load failed · nothing to keep',
    note: 'The status pane replaces the transcript and carries the reason and Retry. Nothing else says it; there is no composer.',
    session: state('initial', { load: 'failed', lastError: SOCKET_ERROR }),
    composer: 'none',
  },
  {
    variant: 'Reconnecting · history in memory',
    note: 'The transcript and the draft stay. The Connecting tail row is the only signal; no bar, no pane.',
    session: state('reconnecting', {
      load: 'reconnecting',
      blocks: shortTranscriptBlocks(),
    }),
    composer: 'default',
  },
  {
    variant: 'Reconnect failed · history in memory',
    note: 'The transcript stays readable. The notice at its tail names the failure with Retry; the composer waits until the session is back.',
    session: state('reconnect-failed', {
      load: 'failed',
      lastError: SOCKET_ERROR,
      blocks: shortTranscriptBlocks(),
    }),
    composer: 'none',
  },
  {
    variant: 'Generation failed · error record',
    note: 'The record at the tail carries the message, the diagnostics, Copy and Retry. The dock adds nothing.',
    session: state('generation', {
      status: 'error',
      lastError: 'Anthropic API request failed with HTTP 529: Overloaded.',
      blocks: [...shortTranscriptBlocks(), generationErrorBlock()],
    }),
    composer: 'default',
  },
  {
    variant: 'Request refused · no record',
    note: 'The server refused a request without writing a record. A notice at the tail says so; there is nothing to retry.',
    session: state('refused', {
      status: 'error',
      lastError: 'The request was refused: another view holds this conversation.',
      blocks: shortTranscriptBlocks(),
    }),
    composer: 'default',
  },
  {
    variant: 'No model can send',
    note: 'Not a failure: a neutral notice replaces the input and opens the settings page.',
    session: state('no-models', { blocks: shortTranscriptBlocks() }),
    composer: 'noModels',
  },
  {
    variant: 'Archived',
    note: 'Not a failure: the same neutral notice with Restore.',
    session: state('archived', {
      archived: true,
      blocks: shortTranscriptBlocks(),
    }),
    composer: 'archived',
  },
]
</script>

<template>
  <div class="space-y-8">
    <GallerySection
      title="Session failures"
      note="The product's ChatSession over fixed states. A failure is named exactly once, in flow: in the status pane when there is nothing to keep, at the transcript's tail when history stays on screen, or in the transcript record when the turn itself failed."
    >
      <div class="grid gap-6 xl:grid-cols-2">
        <GallerySpecimen
          v-for="item in cases"
          :key="item.session.id"
          wide
          :variant="item.variant"
        >
          <div class="space-y-2">
            <p class="text-[12px] leading-4 text-fg-muted">{{ item.note }}</p>
            <div
              class="flex h-[26rem] min-w-0 flex-col overflow-hidden rounded-xl border border-line bg-surface-base"
            >
              <ChatSession
                :conversation="item.session"
                has-provider
                @save-scroll="(_id, value) => (item.session.scroll = value)"
              >
                <template #workspace
                  ><span class="text-chrome text-fg-muted"
                    >Local workspace</span
                  ></template
                >
                <template v-if="item.composer !== 'none'" #composer>
                  <GalleryComposer
                    :conversation-id="item.session.id"
                    placeholder="Ask Demi…"
                    draft="Keep this draft while the session recovers."
                    :providers="item.composer === 'noModels' ? [] : undefined"
                    :models="item.composer === 'noModels' ? {} : undefined"
                    :archived="item.composer === 'archived'"
                  />
                </template>
              </ChatSession>
            </div>
          </div>
        </GallerySpecimen>
      </div>
    </GallerySection>
    <GallerySection
      title="Conversation route"
      note="The page kinds ChatPage shows instead of a session, over the same RegionStatus pane."
    >
      <div class="grid gap-6 xl:grid-cols-2">
        <GallerySpecimen wide variant="Conversation not found · list loaded">
          <div
            class="flex h-[16rem] flex-col overflow-hidden rounded-xl border border-line bg-surface"
          >
            <SessionStatus kind="missing" />
          </div>
        </GallerySpecimen>
        <GallerySpecimen wide variant="Conversation list failed · id unknown">
          <div
            class="flex h-[16rem] flex-col overflow-hidden rounded-xl border border-line bg-surface"
          >
            <SessionStatus kind="failed" />
          </div>
        </GallerySpecimen>
      </div>
    </GallerySection>
  </div>
</template>
