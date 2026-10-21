import type { AgentCost, Block, CacheChart, ConversationAttachment, MessageComment, PendingCall, ProviderFailureFacts, QueuedMessage, SessionPhase, SideQuestion, TurnSpeed, Version } from '@demicodes/protocol'
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
import type { AttachedHost, ConversationMetadata, ConversationShare, ConversationTarget, DeviceKind, FiledRun, LastHourCost, WorkflowChoice } from '../api/generated/web-api'
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
  /** How many blocks of the window lie within the view's reach on either side of that block, which the window holds beyond (`PREFETCH_SCREENS`); absent for none. */
  shownWithin?: number
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
  /** The devices the conversation's agents attached, as a slot under the revision the metadata's hosts carry. */
  attachedHosts: Raw<Slot<AttachedHost[]>>
  /**
   * The descriptions of the attachments its agents' replies name, by the
   * node whose replies show them and then by their ids, such as `a3`: what
   * the transcript's frames and pages and the socket's later uploads bring
   * (`docs/decisions/0029-a-conversation-opens-whole.md`). An attachment
   * never changes, so a description, once held, stands.
   */
  attachmentDescriptions: Record<string, Record<string, ConversationAttachment>>
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
  /**
   * What the conversation cost in the last hour, as the backend counts it
   * while a card shows it
   * (`docs/decisions/0041-the-cost-shows-the-last-hour.md`); null while none
   * does, and until the first read answers.
   */
  lastHour: LastHourCost | null
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
  /** How many blocks of the window lie within the view's reach on either side of that block, which the window holds beyond (`PREFETCH_SCREENS`); absent for none. */
  shownWithin?: number
  failures: Record<string, ProviderFailureFacts>
  sessionPhase: SessionPhase
  pendingCalls: PendingCall[]
  waitingSince: number | null
  /** The page's question, follow-up or Retry on its way (`settleSent`); null for none. */
  sent: SentAction | null
  /** The folds and openings of its card on their way, one at a time; none while none is. */
  folding: SideFolding | null
}

/**
 * A side question's question, follow-up or Retry the page sent, until the
 * conversation's socket shows the server has it.
 */
export interface SentAction {
  /** The transcript the page held when it sent the action. */
  heldWhenSent: HeldTranscript
  /** The backend answered that it took the action. */
  taken: boolean
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
