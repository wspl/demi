<script setup lang="ts">
import { reactive } from 'vue'
import ChatSession from '@demicodes/web-ui/agent/ChatSession.vue'
import type { ChatSessionState } from '@demicodes/web-ui/agent/types'
import SessionStatus from '@demicodes/web-ui/agent/SessionStatus.vue'
import ConnectionBanner from '@demicodes/web-ui/ui/ConnectionBanner.vue'
import type { ConnectionProblem } from '@demicodes/web-ui/transport/connection'
import type { Block } from '@demicodes/protocol'
import type { SentenceText } from '@demicodes/web-ui/ui/ui-text'
import { generationErrorBlock, shortTranscriptBlocks } from '../fixtures/blocks'
import {
  compactedAfterFailedTurnTranscript,
  failedCompactionTranscript,
  stoppedCompactTranscript,
} from '../fixtures/compaction'
import { productWould } from '../product-would'
import { WORKSPACE_ROOT } from '../fixtures/workspace'
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
 * A lost connection is not a failure: the runtime reconnects on its own. The
 * Connecting tail row is all the reader sees when only the conversation's
 * socket was lost; when the page cannot reach the backend at all, the
 * connection banner says so and the transcript adds nothing.
 */
interface SessionCase {
  variant: SentenceText
  note: SentenceText
  session: ChatSessionState
  /** The app's connection banner shows above the session: the backend is away. */
  banner?: ConnectionProblem
  composer: 'default' | 'none' | 'noModels' | 'archived'
}

function state(
  id: string,
  partial: Partial<ChatSessionState> & { blocks?: Block[] },
): ChatSessionState {
  return reactive<ChatSessionState>({
    id: `error-${id}`,
    cwd: WORKSPACE_ROOT,
    title: 'Build a minesweeper game',
    blocks: [],
    queue: [],
    pendingSteers: [],
    phase: 'idle',
    load: 'ready',
    lastError: null,
    pendingAction: null,
    failures: {},
    archived: false,
    scroll: null,
    subagents: [],
    terminals: [],
    ...partial,
  })
}

/** The product would open a new conversation from the answer. */
async function forkFromAnswer(): Promise<void> {
  productWould('Fork the conversation from this answer')
}

/** The backend's own answer to the transcript read: a 500 with its error body, never the connection. */
const LOAD_ERROR = 'The transcript could not be read: database disk image is malformed'

const cases: SessionCase[] = [
  {
    variant: 'Initial load failed · nothing to keep',
    note: 'The backend answered the transcript read with its own error. The status pane replaces the transcript and carries the reason and Retry. Nothing else says it; there is no composer. A read that cannot reach the backend never ends here: it waits under the banner, as the next specimen shows.',
    session: state('initial', { load: 'failed', lastError: LOAD_ERROR }),
    composer: 'none',
  },
  {
    variant: 'Opened while Demi restarts · loading under the banner',
    note: 'Not a failure. The user opened a conversation the page had not loaded while it cannot reach Demi: the banner says so, and the conversation shows it loading until the page is back, then loads. Nothing to click.',
    session: state('opening-away', { load: 'loading' }),
    banner: 'restarting',
    composer: 'default',
  },
  {
    variant: 'Connection lost · reconnecting on its own',
    note: 'Not a failure. Only this conversation’s socket was lost while the page still reaches Demi: the transcript, the draft and the composer stay, and the “Connecting” tail row is the only signal while the runtime retries with backoff. Nothing to click.',
    session: state('reconnecting', {
      load: 'reconnecting',
      blocks: shortTranscriptBlocks(),
    }),
    composer: 'default',
  },
  {
    variant: 'Demi restarting · the banner says it',
    note: 'Not a failure. The page cannot reach Demi, and the banner across the top of the app is the one place that says so: the conversation, whose socket waits for the same backend, adds no “Connecting” row of its own. The transcript, the draft and the composer stay. Nothing to click.',
    session: state('backend-away', {
      load: 'reconnecting',
      blocks: shortTranscriptBlocks(),
    }),
    banner: 'restarting',
    composer: 'default',
  },
  {
    variant: 'Generation failed · error record',
    note: 'The record at the tail carries the message, the diagnostics, Copy and Retry. The dock adds nothing.',
    session: state('generation', {
      lastError: 'Anthropic API request failed with HTTP 529: Overloaded.',
      blocks: [...shortTranscriptBlocks(), generationErrorBlock()],
    }),
    composer: 'default',
  },
  {
    variant: 'Compaction failed inside a turn · Resume',
    note: 'The turn compacted after its answer, and the summary request failed. The record ends the turn, so Resume sits in the dock.',
    session: state('compaction-turn', { blocks: failedCompactionTranscript(false) }),
    composer: 'default',
  },
  {
    variant: 'Compact failed · no turn ended',
    note: 'The user pressed Compact after a finished answer, and the summary request failed. The record stays, but no turn is left unfinished, so the dock offers nothing.',
    session: state('compaction-compact', { blocks: failedCompactionTranscript(true) }),
    composer: 'default',
  },
  {
    variant: 'Compacted after a failed turn · Resume stays',
    note: 'The second message failed, then the user compacted. The divider is the last block, but a compaction ends no turn: the failed turn behind it still offers Resume, which reruns it after the summary.',
    session: state('compaction-after-failure', { blocks: compactedAfterFailedTurnTranscript() }),
    composer: 'default',
  },
  {
    variant: 'Compact stopped · nothing to continue',
    note: 'The user pressed Compact after a finished answer and stopped it while the summary was written. The stop wrote nothing, so no stop record is left and the dock offers no Continue.',
    session: state('compaction-stopped', { blocks: stoppedCompactTranscript() }),
    composer: 'default',
  },
  {
    variant: 'Request refused · no record',
    note: 'The server refused a request without writing a record. A notice at the tail says so; there is nothing to retry.',
    session: state('refused', {
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
      title="Session Failures"
      note="The product’s ChatSession over fixed states. A failure is named exactly once, in flow: in the status pane when there is nothing to keep, at the transcript’s tail when history stays on screen, or in the transcript record when the turn itself failed."
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
              <ConnectionBanner v-if="item.banner" :problem="item.banner" />
              <ChatSession
                :conversation="item.session"
                has-provider
                :backend-away="item.banner !== undefined"
                :fork="forkFromAnswer"
                @retry="productWould('Resume the Turn')"
                @retry-load="productWould('Load the Conversation Again')"
                @rename="item.session.title = $event"
                @retitle="productWould('Detect a Title for the Conversation')"
                @save-scroll="(_id, value) => (item.session.scroll = value)"
              >
                <template #workspace
                  ><span class="text-chrome text-fg-muted"
                    >Local workspace</span
                  ></template
                >
                <template v-if="item.composer !== 'none'" #composer>
                  <GalleryComposer
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
      title="Conversation Route"
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
