import type { AgentCost, Block, CacheChart, MessageComment, PendingCall, ProviderFailureFacts, QueuedMessage, SessionPhase, SideQuestion, TurnSpeed, Version } from '@demicodes/protocol'
import type { ShownError } from '@demicodes/web-ui/infra/errors'
import type { FileBrowserPlatform } from '@demicodes/web-ui/files/types'
import type { DeviceStart } from '@demicodes/web-ui/devices/installation'
import type { AutostartUnavailable } from '@demicodes/web-ui/devices/autostart'
import type { DeviceMark } from '@demicodes/web-ui/devices/mark'
import type { DeviceState } from '@demicodes/web-ui/devices/state'
import type { DeviceReport } from '@demicodes/web-ui/devices/report'
import type {
  SidebarConversation,
  SidebarProject,
} from '@demicodes/web-ui/sidebar/types'
import type { ConversationState, ModelSettings, PendingSteerMessage } from '@demicodes/web-ui/agent/types'
import type {
  ComposerFileAttachment,
  ComposerRemoteAttachment,
} from '@demicodes/web-ui/agent/message-input/attachments'
import type { PendingAction } from '@demicodes/web-ui/agent/activity-slot'
import type { SendWay } from '@demicodes/web-ui/agent/send-way'
import type { SessionLoad } from '@demicodes/web-ui/agent/session-status'
import type { SubagentRecord } from '@demicodes/web-ui/agent/subagents'
import type { TerminalRecord } from '@demicodes/web-ui/agent/terminals'
import type { Raw } from 'vue'
import type { Slot } from '@demicodes/web-ui/infra/slot'
import type { AttachedHost, ConversationMetadata, ConversationShare, ConversationTarget, DeviceKind, FiledRun, WorkflowChoice } from '../api/generated/web-api'
import type { HeldTranscript } from '@demicodes/web-ui/transport/protocol'
import type { NotSaved, RecordSlot } from '../conversation/record'
import type { PersistedScrollState } from '@demicodes/web-ui/composables/useBlockVirtualizer'
import type { SavedDraft, SavedFile } from '../conversation/drafts'
import type { MessageEditState } from '@demicodes/web-ui/agent/message-editing'

export type ProductAttachment =
  | (ComposerFileAttachment & Pick<SavedFile, 'file' | 'upload' | 'claim'>)
  | (ComposerRemoteAttachment & { deviceId: string })

/** A message sent and not yet confirmed, with the way it was sent. */
export type PendingSend = SavedDraft['pendingSends'][number] & { way: SendWay }

/** The sidebar's `device` is derived from `target` and the devices for the row (`App.vue`), never kept. */
export interface Conversation extends Omit<SidebarConversation, 'device'> {
  persistence: 'draft' | 'pending' | 'synced'
  target: ConversationTarget
  revision: number
  readRevision: number
  archived: boolean
  /**
   * The automation's run it is filed under, out of the sidebar until the
   * user answers it or moves it; null otherwise.
   */
  filed: FiledRun | null
  /**
   * What a page shows only of an open conversation, as its socket or a read
   * of it brings it and the page's own changes, such as a model chosen, show
   * over it until they land; its fields below follow it.
   */
  metadata: Raw<Slot<ConversationMetadata>>
  /** The version of the conversation's agents the page holds; null before it holds any. */
  agentsHeld: Version | null
  /** The open tree runs with commands of plugins since turned on or off; a reload would change it. */
  pluginsChanged: boolean
  /** The backend says the agent only waits for the conversation's offline device: a move stops nothing. */
  onlyWaitsForHost: boolean
  /** The speed of the root's last turn, which the chips row shows once no reading does. */
  turnSpeed: TurnSpeed | null
  /** The conversation's shares, its own and its answers'. */
  shares: ConversationShare[]
  createdAt: string
  cwd: string
  /** The parts of the root's transcript the page holds. */
  history: ConversationState['history']
  /** The block whose window the transcript shows; null for the latest. */
  shownAt: string | null
  /** The entries of the root's newest instructions block as a page named them. */
  instructions: ConversationState['instructions']
  /** Each compaction marker's boundary's summary size, by the marker's id, as pages named them. */
  summaries: Record<string, number>
  phase: SessionPhase
  queue: QueuedMessage[]
  pendingSteers: PendingSteerMessage[]
  /** The message the root's running turn started with, until the turn writes it; null otherwise. */
  starting: QueuedMessage | null
  /** The calls the model is writing. */
  pendingCalls: PendingCall[]
  /** When the root began to wait for the model's answer, as `Date.now()`; null when it waits for none. */
  waitingSince: number | null
  /** The output of the request the root agent runs; null when its turn has none. */
  requestOutput: ConversationState['requestOutput']
  model: ModelSettings
  /**
   * The Workflow menu choice of a conversation without a record, which its
   * first send gives it (`crates/backend-user-shard/src/conversation/workflows.rs`); one with
   * a record holds its choice there.
   */
  workflow: WorkflowChoice
  lastError: ShownError | null
  /** The composer's Markdown, a mark where each file's capsule stands. */
  draft: string
  /** Counts the drafts shown from outside, another page's or a restored one, which the editor then shows anew. */
  draftShown: number
  /**
   * The comments waiting to be sent with the next message, in the order
   * they were saved, which belong to the draft as its text does.
   */
  comments: MessageComment[]
  /**
   * The conversation's draft and its messages saved for later, newest first: one slot,
   * which the backend changes in one transaction, of what the page last read of each,
   * with their revisions, and the page's own typing, saves, edits, sends and deletions
   * not landed yet (`record.ts`). The composer shows its draft.
   */
  record: Raw<RecordSlot>
  /** The messages the backend did not save, newest first, which the list shows as Not Saved; only this page has them. */
  notSaved: NotSaved[]
  /** The Saves of saved messages' edits on their way, which the web browser keeps until the backend confirms them. */
  savedWrites: SavedDraft['savedWrites']
  /** Every file the composer carries, those of the message first, in its order. */
  files: ProductAttachment[]
  /** The files the message has, in the order of its capsules; the composer's document says so. */
  attachmentIds: string[]
  /**
   * The messages sent and not yet confirmed, in the order they were sent,
   * each with the way it was sent, which says what it will be while it is
   * delivered; a page that restores them after it closed holds them as messages
   * whose delivery was interrupted.
   */
  pendingSends: PendingSend[]
  /**
   * The waiting messages whose Recall is on its way, by id: a steer, a
   * queued message, or a message on its way that will be one. The list
   * shows none of them from the press.
   */
  removing: string[]
  messageEdit: MessageEditState | null
  scroll: PersistedScrollState | null
  /** The devices the conversation's agents attached, as a slot whose revision is the metadata's `hostsRevision`: metadata with a higher one reads them again. */
  attachedHosts: Raw<Slot<AttachedHost[]>>
  subagents: SubagentRecord[]
  /**
   * The children whose transcripts the page shows, the child its Agents
   * window shows, to whose nodes the socket subscribes besides the root's
   * (`crates/agent-server/src/server/tree.rs`).
   */
  shownSubagents: string[]
  /**
   * Its side questions as its agents name them, each with the transcript
   * the page holds of it, in the order they were asked.
   */
  sides: SideRecord[]
  terminals: TerminalRecord[]
  load: SessionLoad
  pendingAction: PendingAction
  /** The failure facts of the error blocks, by block id, as the backend sends them. */
  failures: Record<string, ProviderFailureFacts>
  contextUsage: ConversationState['contextUsage']
  /**
   * What the root's own calls cost, as its entry says; the conversation's
   * is its agents' with the metadata's `otherCost`
   * (`packages/web-ui/src/agent/context-cost.ts`).
   */
  cost: AgentCost
  /**
   * The cache chart of each agent's node the page subscribes to, by the
   * node, as its subscription and its turn calls bring it
   * (`docs/decisions/0019-the-server-counts-the-cache-chart.md`).
   */
  charts: Record<string, CacheChart>
}

export interface Device extends DeviceReport {
  id: string
  kind: DeviceKind
  name: string
  state: DeviceState
  /** How much of its next runner it has downloaded, while that download moves (`DeviceStatus`). */
  downloadPercent: number | null
  platform: FileBrowserPlatform
  home: string | null
  seen?: string
  /** How to start a paired device's runner again; null for the Cloud. */
  start: DeviceStart | null
  /** Why a paired device will not start its runner by itself after a restart; null when it does. */
  autostartUnavailable: AutostartUnavailable | null
  /** Its mark, with the number it shows (`deviceMarkOf`); null for the Cloud. */
  mark: DeviceMark | null
}

export type Project = SidebarProject & {
  deviceId: string
}

/**
 * One side question of a conversation: its node's entry and what the page
 * holds of its side conversation, which the socket's frames keep live while
 * the page subscribes to its node and the transcript routes read by its
 * node.
 */
export interface SideRecord {
  id: string
  /** What its entry says of it, or the page's own question until an entry names it. */
  record: SideQuestion & { id: string }
  /** Whether an entry of the conversation's agents named it. */
  listed: boolean
  history: HeldTranscript
  /** The block its view shows, which bounds what of its history the page holds; null for the latest. */
  shownAt: string | null
  failures: Record<string, ProviderFailureFacts>
  sessionPhase: SessionPhase
  pendingCalls: PendingCall[]
  waitingSince: number | null
  /** The page's question, follow-up or Retry is on its way. */
  sending: boolean
  /** The folds and openings of its card on their way, one at a time; none while none is. */
  folding: SideFolding | null
}

/**
 * The folds of a side question's card the page sent, one write at a time:
 * the user's latest choice, which the card shows whatever an earlier write
 * or a read answers meanwhile, and what the backend last confirmed, which a
 * refused last write goes back to.
 */
export interface SideFolding {
  intent: boolean
  confirmed: boolean
  writes: number
  chain: Promise<void>
}
