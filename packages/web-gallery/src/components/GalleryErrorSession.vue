<script setup lang="ts">
import { GALLERY_MARKS, shownAmong } from '../fixtures/device-marks'
import { refusalFailure, requestFailure } from '../fixtures/request-failures'
import { wholeHistory } from '@demicodes/web-ui/agent/history'
import { reactive } from 'vue'
import ChatSession from '@demicodes/web-ui/agent/ChatSession.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import type { ChatSessionState } from '@demicodes/web-ui/agent/types'
import { waitClock } from '@demicodes/web-ui/agent/wait-clock'
import SessionStatus from '@demicodes/web-ui/agent/SessionStatus.vue'
import ConnectionBanner from '@demicodes/web-ui/ui/ConnectionBanner.vue'
import type { ConnectionProblem } from '@demicodes/web-ui/transport/connection'
import type { Block } from '@demicodes/protocol'
import type { SentenceText } from '@demicodes/web-ui/ui/ui-text'
import { accountSwitchBlocks, deviceOfflineErrorBlock, generationErrorBlock, notExposedErrorBlock, shortTranscriptBlocks, signedOutErrorBlock, usageLimitErrorBlock, WAITING_SHELL_SCRIPT, waitingShellCall } from '../fixtures/blocks'
import { ago } from '../fixtures/time'
import { galleryMessages } from '../fixtures/send-while-running'
import {
  compactedAfterFailedTurnTranscript,
  failedCompactionTranscript,
  stoppedCompactTranscript,
} from '../fixtures/compaction'
import { productWould } from '../product-would'
import { WORKSPACE_ROOT } from '../fixtures/workspace'
import { demoDeviceStart } from '../fixtures/device-installation'
import type { OfflineHost } from '@demicodes/web-ui/agent/offline-host'
import type { HostChoice, HostDeviceOption, HostMenuHost } from '@demicodes/web-ui/hosts/types'
import GalleryComposer from './GalleryComposer.vue'
import GallerySection from './GallerySection.vue'
import GallerySpecimen from './GallerySpecimen.vue'

/**
 * Every session-level failure, pinned. Each specimen is the product's
 * ChatSession over a fixed state, so what the reader sees here is what the
 * product shows for that state; only a device a failure names toggles, as its
 * runner comes and goes.
 *
 * The rule the specimens demonstrate is that a failure is named once, in flow:
 *
 * - No history to keep: the status pane replaces the transcript (Retry lives
 *   there).
 * - History in memory: the transcript stays, and an ErrorNotice at its tail
 *   names the failure (Retry lives there).
 * - The failure is a transcript record: the record itself carries Retry, and
 *   the dock adds nothing.
 *
 * A lost connection is not a failure: the runtime reconnects on its own. The
 * Connecting tail row is all the reader sees when only the conversation's
 * socket was lost. When the page cannot reach the backend at all, the
 * connection banner says so and the transcript adds nothing.
 */
interface SessionCase {
  variant: SentenceText
  note: SentenceText
  session: ChatSessionState
  /** The app's connection banner shows above the session: the backend is away. */
  banner?: ConnectionProblem
  /**
   * Where the conversation runs now, whose runner the specimen connects and
   * disconnects; the offline card's move changes it.
   */
  primary?: HostMenuHost
  composer: 'default' | 'none' | 'noModels' | 'archived'
  /** The port the failure names is exposed by now. */
  exposed?: boolean
}

/** The user's devices, as the offline card's move lists them. */
const errorDevices: HostDeviceOption[] = shownAmong([
  { id: 'mac', name: 'MacBook Pro', state: 'online', path: 'thisComputer', mark: GALLERY_MARKS.mac },
  { id: 'studio', name: 'Studio PC', state: 'online', path: 'localNetwork', mark: GALLERY_MARKS.windows },
])

/** The primary Host's card above the dock's chips, while it is an offline device. */
function offlineCard(primary: HostMenuHost | undefined): OfflineHost | null {
  if (primary?.kind !== 'device' || primary.state === 'online') {
    return null
  }
  return {
    name: primary.name,
    start: demoDeviceStart('macos'),
    primaryHost: primary,
    devices: errorDevices.map((device) => device.id === primary.id ? { ...device, state: 'offline' } : device),
  }
}

/**
 * The offline card moved the conversation: it runs on the chosen Host now,
 * online, and each command that waited for the device it left ends without
 * having run, saying where the conversation runs now.
 */
function move(item: SessionCase, host: HostChoice, name: string): void {
  if (!item.primary) {
    return
  }
  const left = item.primary.name
  for (const terminal of item.session.terminals) {
    if (terminal.waitingFor !== undefined) {
      delete terminal.waitingFor
      Object.assign(terminal, {
        phase: 'exited',
        exitCode: 127,
        endedAt: new Date().toISOString(),
        output: `This conversation now runs on ${name}; this command did not run, since ${left} was offline.\n`,
      })
    }
  }
  Object.assign(item.primary, { id: host.kind === 'cloud' ? 'cloud' : host.id, kind: host.kind, name, state: 'online' })
}

/**
 * The user's Stop of the specimen's running turn: a command that waited for
 * the device ends stopped without having run, and its call with it, as the
 * product records the stop.
 */
function stopTurn(item: SessionCase): void {
  for (const terminal of item.session.terminals) {
    if (terminal.waitingFor !== undefined) {
      delete terminal.waitingFor
      Object.assign(terminal, { phase: 'aborted', endedAt: new Date().toISOString() })
    }
  }
  for (const window of item.session.history.windows) {
    for (const block of window.blocks) {
      if (block.type === 'tool_call' && block.status === 'executing') {
        Object.assign(block, { status: 'error', endedAt: new Date().toISOString() })
      }
    }
  }
  item.session.phase = 'idle'
}

/**
 * The specimen's runner connected or disconnected: a command that waited for
 * the device starts once its runner is back.
 */
function toggleRunner(item: SessionCase): void {
  if (!item.primary) {
    return
  }
  item.primary.state = item.primary.state === 'online' ? 'offline' : 'online'
  if (item.primary.state === 'online') {
    for (const terminal of item.session.terminals) {
      delete terminal.waitingFor
    }
  }
}

function state(
  id: string,
  { blocks = [], ...partial }: Partial<ChatSessionState> & { blocks?: Block[] },
): ChatSessionState {
  return reactive<ChatSessionState>({
    id: `error-${id}`,
    cwd: WORKSPACE_ROOT,
    title: 'Build a minesweeper game',
    history: wholeHistory(blocks),
    shownAt: null,
    summaries: {},
    queue: [],
    pendingSteers: [], starting: null,
    pendingCalls: [],
    waitingSince: null, waitAction: null, requestOutput: null, turnSpeed: null,
    phase: 'idle',
    load: 'ready',
    lastError: null,
    pendingAction: null,
    waitClock: waitClock(),
    failures: {},
    archived: false,
    scroll: null,
    subagents: [],
    terminals: [],
    ...partial,
  })
}

/** When the session limit the usage-limit specimen hit resets: two hours on, on the hour. */
const HOUR_MS = 3_600_000
const limitResets = new Date((Math.floor(Date.now() / HOUR_MS) + 2) * HOUR_MS).toISOString()
/** The same limit, hit yesterday: its time has passed. */
const limitReset = new Date((Math.floor(Date.now() / HOUR_MS) - 20) * HOUR_MS).toISOString()

/** The product would open a new conversation from the answer. */
async function forkFromAnswer(): Promise<void> {
  productWould('Fork the conversation from this answer')
}

/**
 * Each specimen's scroll as the product saves it when the reader leaves:
 * kept for a next open, never fed back into the session on screen, whose
 * list would otherwise restore a place taken before its dock was measured.
 */
const savedScrolls = reactive<Record<string, ChatSessionState['scroll']>>({})

const cases: SessionCase[] = [
  {
    variant: 'Initial load failed · nothing to keep',
    note: 'The backend answered the transcript read with its own error, here that it could not read the conversation’s database. The status pane replaces the transcript and carries the reason, in the page’s words, and Retry; the backend’s message, which names its files, goes to the console. Nothing else says it; there is no composer. A read that cannot reach the backend never ends here: it waits under the banner, as the next specimen shows.',
    session: state('initial', { load: 'failed', lastError: refusalFailure('conversation_unreadable') }),
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
      lastError: { message: 'Anthropic API request failed with HTTP 529: Overloaded.', requestId: null },
      blocks: [...shortTranscriptBlocks(), generationErrorBlock()],
    }),
    composer: 'default',
  },
  {
    variant: 'Usage limit · Resume at its time',
    note: 'A Claude subscription’s session limit refused the turn. The record says when the limit resets and carries the one switch that belongs to it: Resume at that time, the user’s preference for every such turn, the same as Settings › General’s Resume after a usage limit resets, so ticking it here turns that on too. Resume in the dock goes on now.',
    session: state('usage-limit', {
      blocks: [...shortTranscriptBlocks(), usageLimitErrorBlock('usage-limit-error', limitResets)],
      failures: { 'usage-limit-error': { retryAt: limitResets } },
    }),
    composer: 'default',
  },
  {
    variant: 'Usage limit · Continue with another account',
    note: 'The entry has more accounts and no rotation: beside Resume, the dock offers to continue with the account that has the most room now, which becomes the active account in Settings and resumes the turn there.',
    session: state('usage-limit-other', {
      blocks: [...shortTranscriptBlocks(), usageLimitErrorBlock('usage-limit-other-error', limitResets)],
      failures: {
        'usage-limit-other-error': {
          retryAt: limitResets,
          continueWith: { credentialId: 'a2', label: 'sam@example.com' },
        },
      },
    }),
    composer: 'default',
  },
  {
    variant: 'Signed out · Open Providers',
    note: 'Claude refused the account’s sign-in and its command-line tool could not renew it, so Demi marked the account signed out. The record says so in its own words rather than the generic sign-in failure, and the dock offers Open Providers, where the account’s row offers Sign In Again. Once it is signed in again, the record reads as the generic sign-in failure.',
    session: state('signed-out', {
      blocks: [...shortTranscriptBlocks(), signedOutErrorBlock('signed-out-error')],
      failures: { 'signed-out-error': { retryAt: null, signedOut: { provider: 'Claude Code', account: 'alex@example.com' } } },
    }),
    composer: 'default',
  },
  {
    variant: 'Port not exposed · Expose Port',
    note: 'The entry’s endpoint names port 11434 of the user’s Mac by its id, and the Mac has not exposed it, so the request was refused before it reached the Mac. The record says so in Demi’s words, and the dock offers Expose Port beside Resume: it exposes the port, as Settings › Devices lists it, and resumes the turn.',
    session: state('not-exposed', {
      blocks: [...shortTranscriptBlocks(), notExposedErrorBlock('not-exposed-error')],
      failures: { 'not-exposed-error': { retryAt: null, notExposed: { deviceId: 'mac', device: 'alex-mbp', port: 11434 } } },
    }),
    composer: 'default',
  },
  {
    variant: 'Port exposed since · Resume',
    note: 'The same failure after the port was exposed from Settings › Devices or another tab, which the page learns of when it happens: the record still says what happened, and the dock offers Resume alone, which makes the request again.',
    session: state('exposed-since', {
      blocks: [...shortTranscriptBlocks(), notExposedErrorBlock('exposed-since-error')],
      failures: { 'exposed-since-error': { retryAt: null, notExposed: { deviceId: 'mac', device: 'alex-mbp', port: 11434 } } },
    }),
    composer: 'default',
    exposed: true,
  },
  {
    variant: 'Device offline · named, after its retries',
    note: 'The entry’s endpoint names a port of the user’s Mac while the Mac is offline. The request was retried as one to an unreachable endpoint is, and the record names the Mac, since the next step is to bring it online; Resume tries again.',
    session: state('device-offline', {
      blocks: [...shortTranscriptBlocks(), deviceOfflineErrorBlock('device-offline-error')],
      failures: { 'device-offline-error': { retryAt: null, deviceOffline: { deviceId: 'mac', device: 'alex-mbp' } } },
    }),
    composer: 'default',
  },
  {
    variant: 'Account rotation · the agent moved to another account',
    note: 'Not a failure. With In Order or Balanced, the agent’s account reached its limit before anything of the answer showed: the same request went at once to the next account, the transcript says so in a neutral line, with the reset in the viewer’s time zone, and the answer follows. Nothing to click.',
    session: state('account-switch', {
      blocks: [...shortTranscriptBlocks(), ...accountSwitchBlocks(limitResets)],
    }),
    composer: 'default',
  },
  {
    variant: 'Usage limit · its time has passed',
    note: 'The same failure after the limit reset: the record still says when it reset, but the switch is gone, since there is no time left to resume at. Resume in the dock goes on.',
    session: state('usage-limit-passed', {
      blocks: [...shortTranscriptBlocks(), usageLimitErrorBlock('usage-limit-passed-error', limitReset)],
      failures: { 'usage-limit-passed-error': { retryAt: limitReset } },
    }),
    composer: 'default',
  },
  {
    variant: 'Host went offline · the agent waits for the device',
    note: 'Not a failure. The conversation’s device went offline while the agent was about to run its tests: the command waits for the device, its row saying Waiting for MacBook Pro, and the turn stays running. Connect Runner starts the command, which then runs as any command does. Move to Another Host… on the device’s offline card moves the conversation at once: the waiting command never ran, it ends saying where the conversation runs now, and the agent goes on there. Stop ends the wait.',
    session: state('host-offline', {
      blocks: [...shortTranscriptBlocks(), waitingShellCall()],
      phase: 'running',
      terminals: [{
        id: 'term-waiting',
        title: 'Run the tests',
        script: WAITING_SHELL_SCRIPT,
        phase: 'running',
        startedAt: ago(20_000),
        output: '',
        toolUseId: waitingShellCall().toolUseId,
        waitingFor: 'MacBook Pro',
      }],
    }),
    primary: reactive<HostMenuHost>({ id: 'mac', name: 'MacBook Pro', kind: 'device', state: 'offline', mark: GALLERY_MARKS.mac }),
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
      lastError: requestFailure('The request was refused: another view holds this conversation.'),
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
              <ChatSession @show-subagents="() => {}"
                :conversation="item.session"
                has-provider
                :backend-away="item.banner !== undefined"
                :fork="forkFromAnswer"
                :primary-host="item.primary"
                v-model:resume-at-retry="galleryMessages.resumeAtRetry"
                @retry="productWould('Resume the Turn')"
                @continue-with="(_provider, account) => productWould(`Make ${account === 'a2' ? 'sam@example.com' : account} the Active Account and Resume the Turn`)"
                @open-providers="productWould('Open Settings › Providers')"
                :is-exposed="() => item.exposed === true"
                @expose-port="(_device, port) => productWould(`Expose Port ${port} of alex-mbp and Resume the Turn`)"
                @retry-load="productWould('Load the Conversation Again')"
                @rename="item.session.title = $event"
                @save-scroll="(id, value) => (savedScrolls[id] = value)"
              >
                <template #workspace
                  ><span class="text-chrome text-fg-muted"
                    >Local workspace</span
                  ></template
                >
                <template v-if="item.composer !== 'none'" #composer>
                  <GalleryComposer
                    draft="Keep this draft while the session recovers."
                    :providers="item.composer === 'noModels' ? [] : undefined"
                    :models="item.composer === 'noModels' ? {} : undefined"
                    :archived="item.composer === 'archived'"
                    :offline-host="offlineCard(item.primary)"
                    :running="item.session.phase === 'running'"
                    @stop="stopTurn(item)"
                    @moved="(host, name) => move(item, host, name)"
                  />
                </template>
              </ChatSession>
            </div>
            <Button
              v-if="item.primary"
              size="sm"
              variant="ghost"
              @click="toggleRunner(item)"
            >
              {{ item.primary.state === 'online' ? 'Disconnect Runner' : 'Connect Runner' }}
            </Button>
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
