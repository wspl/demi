import { computed, markRaw, reactive, type Raw, ref, toRaw, watch } from 'vue'
import { defineStore } from 'pinia'
import { SerialQueue, deferred, moveBefore } from '@demicodes/utils'
import type { HeadlineText, TitleText } from '@demicodes/web-ui/ui/ui-text'
import type { ClientContent, MessageComment } from '@demicodes/protocol'
import { inReadingOrder, type ReplyCommenting } from '@demicodes/web-ui/agent/comments/comments'
import { ConversationCache, type CachedConversation } from '@demicodes/web-ui/agent/conversation-cache'
import { latestBlocks, type TranscriptReveal } from '@demicodes/web-ui/agent/history'
import { conversationTitle } from '@demicodes/web-ui/agent/conversation-title'
import { ConversationRuntime, RemovalRefusedError, isRecordedTurnFailure } from '@demicodes/web-ui/agent/conversation-runtime'
import {
  regenerateMessage,
  restoreMessageEdit,
  sentEditRequest,
  submitMessageEdit,
  type MessageEditHost,
  type MessageEditRequest,
} from '@demicodes/web-ui/agent/message-editing'
import { coded, errorCode, errorMessage, reportError, requestIdOf, shownError, type ReportErrorOptions } from '@demicodes/web-ui/infra/errors'
import { t as shared } from '@demicodes/web-ui/locales/text'
import { ConversationSocketError } from '@demicodes/web-ui/transport/conversation-socket'
import { transcriptShown } from '@demicodes/web-ui/agent/session-status'
import { dismissToast, showToast } from '@demicodes/web-ui/infra/toast'
import { Slot } from '@demicodes/web-ui/infra/slot'
import { showArchived, updateArchived } from '@demicodes/web-ui/sidebar/archived-toast'
import { showRefused, type Refusal, type RefusedAction } from '@demicodes/web-ui/sidebar/refused-toast'
import { forkConversation } from '../api/message-fork'
import type { MessageForkRequest } from '@demicodes/web-ui/agent/message-fork'
import {
  applyModelChange,
  composerModel,
  initialModelSettings,
  lookupSelectedModel,
  offeredSettings,
  type ModelSettings,
  type ModelSettingsChange,
} from '@demicodes/web-ui/agent/model-selection'
import { hasAcceptedSubmission } from '@demicodes/web-ui/agent/submission'
import { runningSubagents, subagentJobFields } from '@demicodes/web-ui/agent/subagents'
import { workToStop } from '@demicodes/web-ui/hosts/move-question'
import { runningTerminals } from '@demicodes/web-ui/agent/terminals'
import { agentWorks, chooseSendWay, turnStartingMessage, type SendWay } from '@demicodes/web-ui/agent/send-way'
import {
  composerAttachmentFromFile,
  composerRemoteAttachment,
  isComposerFile,
} from '@demicodes/web-ui/agent/message-input/attachments'
import {
  contentCopy,
  missingFilesToast,
  pasteMessage,
  type CopiedDeviceFile,
  type CopiedMessage,
  type CopiedStoredFile,
  type PasteHost,
} from '@demicodes/web-ui/agent/message-input/copied-message'
import { composerCapsule } from '@demicodes/web-ui/agent/message-editor/capsules'
import { showRecalled } from '@demicodes/web-ui/agent/recalled-toast'
import { ATTACHMENT_MARK, parseUserMarkdown, serializeUserMarkdown } from '@demicodes/web-ui/markdown/user-markdown'
import { connectConversationClient, takeConnection } from '@demicodes/web-ui/transport/conversation-socket'
import { loadDraft } from '../api/drafts'
import { REQUEST_ID_HEADER, apiRequest, jsonBody, readResponse } from '../api/client'
import {
  attachedHostsSchema,
  batchAnswerSchema,
  createdConversationSchema,
  conversationUpdateSchema,
  subagentsSchema,
  transcriptPageSchema,
  type AttachedHost,
  type AttachedHosts,
  type ConversationBatch,
  type ConversationDraft,
  type ConversationPatch,
  type ConversationStatus,
  type ConversationSummary,
  type CreateConversation,
  type DraftFile,
  type ProductState,
  type ReadRequest,
  type SidebarReorder,
  type Subagents,
  type TranscriptPage,
  type WorkflowChoice,
} from '../api/generated/web-api'
import { isEmptyMessage, joinMessageContent } from '@demicodes/web-ui/agent/message-input/message-content'
import { startingChoices, startingWorkflow, type RememberedChoices } from '@demicodes/web-ui/agent/starting-choices'
import type { WorkflowList } from '@demicodes/web-ui/agent/workflow-menu'
import { blobPicture, createConversationUploads, sendingFileIds } from './uploads'
import { useResources } from '../state/resources'
import { executionFor } from '../targets/execution'
import { useProduct, type ProductChange } from '../state/product'
import { usePreferences } from '../state/preferences'
import { useSession } from '../auth/session'
import type { Conversation, PendingSend, ProductAttachment } from '../state/types'
import { applyConversationEvent, updateLiveStatus } from './activity'
import { composerWriting, createRecordSync, newRecord, type Writing } from './record'
import { createConversationHistoryReads } from './history'
import { findCallBlock, shellCallOf } from './terminals'
import { createConversationCopies } from './offline'
import { readConversationCopy, type AgentCopy } from '../state/offline'
import { t } from './locales/text'
import { EMPTY_TRANSCRIPT, addPage, heldBlocks, heldTranscript } from '@demicodes/web-ui/transport/protocol'
import type { TerminalRecord } from '@demicodes/web-ui/agent/terminals'
import {
  deleteDraft,
  readDraft,
  readLocalDrafts,
  takeLastWords,
  UnreadableDraftError,
  writeDraft,
  writeLastWords,
  type SavedDraft,
  type SavedFile,
} from './drafts'

/**
 * A new conversation whose draft holds nothing yet, no character and no
 * file: the sidebar lists it only while it is open, and leaving it drops it.
 */
function isEmptyDraft(conversation: Conversation): boolean {
  return conversation.persistence === 'draft' && isEmptyMessage(conversation.draft, conversation.attachmentIds.length)
}

/**
 * What Recall did with a message the session held, until the session
 * answers its removal: the composer it refilled, with what it put there, or
 * the toast whose Undo sends the message again.
 */
type TakenBack =
  | { kind: 'refilled'; text: string; fileIds: string[]; comments: MessageComment[] }
  | { kind: 'recalled'; toast: string }

/** `state` with the conversation `id` as `change` makes it; one that is gone stays gone. */
function withConversation(
  state: ProductState,
  id: string,
  change: (record: ConversationSummary) => ConversationSummary,
): ProductState {
  return {
    ...state,
    conversations: state.conversations.map((record) => record.id === id ? change(record) : record),
  }
}

export const useConversations = defineStore('conversations', () => {
  const product = useProduct()
  const preferences = usePreferences()
  const resources = useResources()
  const session = useSession()
  const items = ref<Conversation[]>([])
  /**
   * The conversations this page had a record of that were deleted since, by
   * this page or another; a page that shows one goes to a new conversation.
   */
  const deleted = reactive(new Set<string>())
  /**
   * The conversations with a change the page shows in progress until the
   * backend answers, as a move to another Host (`web-application.md`
   * § Responding to the user, What it does not).
   */
  const pendingChanges = ref<string[]>([])
  /**
   * The archive or restore of each conversation on its way, which its
   * opening and its socket's connections wait for: the backend refuses an
   * archived conversation's socket.
   */
  const archiving = new Map<string, Promise<void>>()
  /**
   * This web browser's own drafts, new conversations before their first
   * send, are on their way back from IndexedDB (`restoreLocalDrafts`): the
   * list is not whole, and an address may name one of them.
   */
  const restoringLocalDrafts = ref(false)
  const listStatus = computed(() => restoringLocalDrafts.value ? 'loading' : product.load)
  /**
   * What the sidebar lists: every conversation not archived; a new
   * conversation whose draft holds nothing yet only while it is the open
   * one, so it shows the moment it opens and goes when it is left empty
   * (`conversations.md` § Conversations and projects).
   */
  // A run's conversation filed under its automation stays out of the sidebar (`automations.md` § Runs).
  const listed = computed(() => items.value.filter((item) =>
    !item.archived && !item.filed && (!isEmptyDraft(item) || item.id === product.activeConversationId)))
  /**
   * The writes on their way, by the conversation they change, or by
   * `conversation_order` for a move: each conversation's go one at a time in
   * order, and a write of several waits for each of them
   * (`web-application.md` § Responding to the user).
   */
  const writes = new SerialQueue()
  // Reactive: words sent to a conversation wait for its draft to be restored (`appendToDraft`).
  const restored = reactive(new Set<string>())
  /** Each conversation's draft as this page last saved or restored it, in the shape `changedDraft` compares. */
  const savedDrafts = new Map<string, string>()
  const uploads = createConversationUploads(saveDrafts, (error) =>
    report(t('failures.uploadAttachment'), error),
  )
  const { uploadInComposer, addFiles, removeFile, releaseSpare, arrangeFiles, retryFile } = uploads
  const recordSync = createRecordSync({
    show: showWriting,
    persist: persistConversation,
    report,
  })
  let lifetime = new AbortController()
  const cache = new ConversationCache()
  /** The history a conversation's page reads beyond its opening (`web-application.md` § Transcript windows). */
  const history = createConversationHistoryReads(
    () => lifetime.signal,
    (conversation) => cache.get(conversation.id)?.runtime != null,
  )
  /** The copies of the conversations opened here, for opening offline (`web-application.md` § Installing and opening offline). */
  const copies = createConversationCopies(() => items.value, () => session.user?.id ?? null)
  /** The commands whose call the page has gone to read whole, each once. */
  const callReads = new WeakSet<TerminalRecord>()

  /**
   * Gives a command without its script its call's title and script
   * (`web-api-conversations.md` § Light form): a call the page holds light, with its input
   * cut, is read whole by its id; a call the page does not hold is found by
   * the command's id first. A call the page holds whole names the command
   * from its frames already.
   */
  async function readCommandCall(conversation: Conversation, terminal: TerminalRecord): Promise<void> {
    const record = toRaw(terminal)
    if (record.script !== undefined || callReads.has(record)) {
      return
    }
    const node = record.subagentId ?? null
    const blocks = heldBlocks(node === null
      ? conversation.history
      : conversation.subagents.find((agent) => agent.id === node)?.history ?? EMPTY_TRANSCRIPT)
    const held = record.toolUseId === undefined ? undefined : findCallBlock(blocks, record.toolUseId)
    if (held && !held.light) {
      return
    }
    callReads.add(record)
    try {
      const at = held ? { node, blockId: held.id } : await history.findCommand(conversation, record.id)
      const call = shellCallOf(await history.readWhole(conversation, at.node, at.blockId))
      if (call && terminal.script === undefined) {
        terminal.title = call.title
        terminal.script = call.script
      }
    } catch (error) {
      // The tab keeps the command's id as its title and shows its output
      // without the prompt line; only its name is missing.
      console.warn('The command\'s call was not read', error)
    }
  }

  /**
   * Counts the times New gave the user the draft already shown, whose
   * composer then takes the focus again.
   */
  const composerFocusRequests = ref(0)
  /**
   * The block a search result opened its conversation at, or a report's
   * title went to, which the conversation's page brings into view and marks
   * once, then clears.
   */
  const reveal = ref<(TranscriptReveal & { conversationId: string }) | null>(null)
  /**
   * The deliveries of the messages sent, by conversation: each
   * conversation's go one at a time, in the order they were sent, so a
   * message never overtakes the one before it (`turns.md` § Steer or
   * queue).
   */
  let deliveries = new SerialQueue()
  /**
   * Recall or Send Now pressed on a steer or a queued message still on
   * its way, by its message id: the user's latest wish, carried out once the
   * session holds the message (`web-application.md` § Responding to the
   * user).
   */
  const wishes = new Map<string, 'recall' | 'sendNow'>()
  /**
   * The messages on their way that a comment's Send Now made, each holding
   * that comment alone: one not delivered leaves no failed message, its
   * comment going back to the waiting ones (`comments-and-ask.md` § Comments).
   */
  const commentSends = new Set<string>()
  /** The draft storage failures this page reported, each once. */
  const storageReported = new Set<'save' | 'restore' | 'unreadable'>()

  // A failed operation is a toast; server state is never replaced by a message.
  function report(title: HeadlineText, error: unknown, options: ReportErrorOptions = {}): void {
    if (error instanceof DOMException && error.name === 'AbortError') {
      return
    }
    if (!lifetime.signal.aborted) {
      reportError(title, error, { ...options, userVisible: true })
    }
  }

  /**
   * Reports, once a page, that the draft storage failed to save or restore,
   * or that a record it holds did not read and was set aside
   * (`web-application.md` § Persistence and adapters). The toast says it in
   * words; what failed goes to the console.
   */
  function storageError(error: unknown, what: 'save' | 'restore'): void {
    const kind = error instanceof UnreadableDraftError ? 'unreadable' : what
    if (storageReported.has(kind)) {
      return
    }
    storageReported.add(kind)
    const detail = error instanceof UnreadableDraftError ? error.detail : undefined
    switch (kind) {
      case 'save':
        reportError(t('failures.saveDrafts'), error, {
          userVisible: true,
          message: t('drafts.storeFailed'),
        })
        break
      case 'restore':
        reportError(t('failures.restoreDrafts'), error, {
          userVisible: true,
          message: t('drafts.readFailed'),
        })
        break
      case 'unreadable':
        reportError(t('failures.restoreStoredDraft'), error, {
          userVisible: true,
          detail,
          message: t('drafts.unreadable'),
        })
        break
    }
  }

  function summaryStatus(
    status: ConversationStatus,
  ): Conversation['status'] {
    if (status === 'running' || status === 'compacting') {
      return 'active'
    }
    // A turn the process died under is a failure, like a provider's; only the
    // user's own Stop is an abort (`turns.md` § Recovering an unfinished turn).
    if (status === 'error' || status === 'interrupted') {
      return 'error'
    }
    if (status === 'stopped') {
      return 'aborted'
    }
    return status === 'completed' ? 'done' : 'idle'
  }

  function metadata(
    record: Pick<
      ConversationSummary,
      | 'id'
      | 'title'
      | 'pinned'
      | 'archived'
      | 'automation'
      | 'target'
      | 'revision'
      | 'readRevision'
      | 'unread'
      | 'cwd'
      | 'pluginsChanged'
      | 'onlyWaitsForHost'
      | 'shares'
      | 'unreadPermissionRequests'
      | 'createdAt'
      | 'updatedAt'
    >,
  ) {
    return {
      id: record.id,
      title: record.title,
      pinned: record.pinned,
      archived: record.archived,
      filed: record.automation,
      target: record.target,
      projectId:
        record.target.kind === 'workspace' ? record.target.workspaceId : null,
      cwd: record.cwd,
      revision: record.revision,
      readRevision: record.readRevision,
      unread: record.unread,
      pluginsChanged: record.pluginsChanged,
      onlyWaitsForHost: record.onlyWaitsForHost,
      shares: record.shares,
      // The sidebar's mark settles once the request is read, though it still waits.
      needsYou: record.unreadPermissionRequests > 0,
      createdAt: record.createdAt,
      updatedAt: record.updatedAt,
    }
  }

  /**
   * A conversation shows the model settings its record holds, whichever page
   * or device changed them last (`web-api-conversations.md` § Sidebar mutations and read
   * state). A record without a model leaves the page's own settings, which a
   * send writes to it.
   */
  function followRecordModel(
    conversation: Conversation,
    record: Pick<ConversationSummary, 'model'>,
  ): void {
    if (record.model) {
      conversation.model = { ...record.model }
    }
  }

  /**
   * The settings the composer shows: the conversation's, with the first
   * usable model while it has none chosen.
   */
  function shownSettings(conversation: Conversation): ModelSettings {
    const pick = composerModel(
      resources.providerInfos,
      resources.models,
      conversation.model.providerId,
      conversation.model.modelId,
    )
    if (!pick.providerId || !pick.modelId) {
      return conversation.model
    }
    return { ...conversation.model, providerId: pick.providerId, modelId: pick.modelId }
  }

  /** `slot`, a conversation's own, for which a read of revision `revision` is on its way or not needed. */
  function askedAt<V>(slot: Slot<V>, revision: number): Raw<Slot<V>> {
    slot.ask(revision)
    // The conversation is reactive; the slot keeps its own refs.
    return markRaw(slot)
  }

  /** A local draft has no backend record yet, so no resolved `cwd`; the first send brings it. */
  function newConversation(record: Omit<ConversationSummary, 'cwd'> & { cwd?: string }): Conversation {
    return {
      ...metadata({ ...record, cwd: record.cwd ?? '' }),
      persistence: 'synced',
      status: summaryStatus(record.status),
      history: EMPTY_TRANSCRIPT,
      shownAt: null,
      instructions: [],
      summaries: {},
      phase: 'idle',
      queue: [],
      pendingSteers: [],
      pendingCalls: [],
      waitingSince: null,
      model: record.model ? { ...record.model } : initialModelSettings(),
      workflow: record.workflow,
      lastError: null,
      draft: '',
      draftShown: 0,
      comments: [],
      record: newRecord(),
      notSaved: [],
      files: [],
      attachmentIds: [],
      pendingSends: [],
      removing: [],
      messageEdit: null,
      scroll: null,
      // Its opening reads the hosts at least this new.
      attachedHosts: askedAt(new Slot<AttachedHost[]>([]), record.hostsRevision),
      subagents: [],
      shownSubagents: [],
      sides: [],
      sidesHeld: null,
      terminals: [],
      load: 'loading',
      pendingAction: null,
      failures: {},
      contextUsage: null,
    }
  }

  /**
   * Keeps everything the conversation shows as its archive flag changes
   * (`conversations.md` § Conversations and projects): archived, it lets its
   * socket go, which the backend would close; brought back, it connects
   * again from the blocks it holds once the restore lands, or, opened while
   * archived, starts its session then. Nothing is read again; a change of
   * its draft held back while it was archived is saved once the restore
   * lands.
   */
  function followArchive(conversation: Conversation): void {
    if (!conversation.archived) {
      void Promise.resolve(archiving.get(conversation.id)).then(() => {
        if (!conversation.archived) {
          recordSync.changed(conversation)
        }
      })
    }
    const entry = cache.get(conversation.id)
    if (!entry) {
      return
    }
    if (conversation.archived) {
      entry.runtime?.suspend()
      return
    }
    if (entry.runtime) {
      void entry.runtime.connect().catch(() => {
        // A refused connection is the conversation's load state, which the
        // page shows with Retry; a suspend meanwhile ends it with nothing to say.
      })
      return
    }
    void entry.opening.then(async () => {
      if (cache.get(conversation.id) === entry && !entry.runtime && !conversation.archived) {
        await startSession(conversation, entry, null)
      }
    }).catch((error: unknown) => {
      // A failed opening already says so; a session that did not start says
      // so as an opening does.
      if (cache.get(conversation.id) === entry && !entry.controller.signal.aborted) {
        failOpening(conversation, error)
      }
    })
  }

  // Each part the channel or a write's answer changes reaches the list at
  // once, in the same step, so what follows a write sees it.
  watch(
    () => product.snapshot,
    (snapshot) => {
      if (!snapshot) {
        return
      }
      const previous = items.value
      for (const item of previous) {
        if (item.persistence === 'synced' && !snapshot.conversations.some((record) => record.id === item.id)) {
          deleted.add(item.id)
        }
      }
      for (const record of snapshot.conversations) {
        deleted.delete(record.id)
      }
      // New conversations before their first send, which only this page
      // has; they keep the front, in their own order.
      const local = previous.filter(
        (item) =>
          item.persistence !== 'synced' &&
          !snapshot.conversations.some((record) => record.id === item.id),
      )
      const synced = snapshot.conversations.map((record) => {
        const current = items.value.find((item) => item.id === record.id)
        if (!current) {
          return newConversation(record)
        }
        if (current.persistence !== 'synced') {
          return current
        }
        const archiveChanged = current.archived !== record.archived
        Object.assign(current, metadata(record))
        followRecordModel(current, record)
        const cached = cache.get(current.id)
        if (!cached?.runtime?.connected) {
          current.status = summaryStatus(record.status)
        }
        // Another page changed the draft or the messages saved for later
        // since this one read them; a conversation whose draft was never
        // read, or that was not opened, reads them when it opens.
        if (current.record.revision('draft') >= 0 && current.record.ask(record.draftRevision, 'draft')) {
          void recordSync.readDraft(current).catch((error) => report(t('failures.readDraft'), error))
        }
        if (cache.get(current.id) && current.record.ask(record.savedRevision, 'saved')) {
          void recordSync.readSaved(current).catch((error) => report(t('failures.readSaved'), error))
        }
        // Another page or a host's shell changed the hosts since this one
        // read them; a conversation not opened reads them when it opens.
        if (current.attachedHosts.ask(record.hostsRevision) && cache.get(current.id)) {
          void readHosts(current).catch((error) => report(t('failures.readHosts'), error))
        }
        if (archiveChanged) {
          followArchive(current)
        }
        return current
      })
      const next = [...local, ...synced]
      items.value = next
      cache.retain(new Set(next.map((item) => item.id)))
      // Reads started for an address the state does not name are nobody's.
      for (const id of earlyReads.keys()) {
        if (!next.some((item) => item.id === id)) {
          earlyReads.delete(id)
        }
      }
      openListed()
    },
    { flush: 'sync' },
  )

  function persisted(conversation: Conversation): SavedDraft {
    const local = conversation.persistence !== 'synced'
    // The web browser keeps the composer's text only while the backend does not
    // have it: a new conversation's, or a change not yet confirmed.
    const unsaved = local || recordSync.unsaved(conversation)
    const sending = sendingFileIds(conversation)
    const files = [
      ...(unsaved ? messageFiles(conversation) : []),
      ...conversation.files.filter((file) => sending.has(file.id)),
    ]
    return {
      messageEdit: conversation.messageEdit
        ? structuredClone(toRaw(conversation.messageEdit))
        : null,
      pendingSends: conversation.pendingSends.map((pending) => structuredClone(toRaw(pending))),
      local:
        conversation.persistence === 'synced'
          ? null
          : {
              phase: conversation.persistence,
              conversation: {
                id: conversation.id,
                title: conversation.title,
                pinned: conversation.pinned,
                archived: conversation.archived,
                target: { ...conversation.target },
                createdAt: conversation.createdAt,
                updatedAt: conversation.updatedAt,
              },
            },
      base: !local && unsaved ? recordSync.base(conversation) : null,
      text: unsaved ? conversation.draft : '',
      comments: unsaved ? conversation.comments.map((comment) => structuredClone(toRaw(comment))) : [],
      notSaved: conversation.notSaved.map((row) => structuredClone(toRaw(row))),
      // A conversation with a record holds its model settings there.
      model: local ? { ...conversation.model } : null,
      // And its Workflow menu choice; Default goes unwritten.
      ...(local && conversation.workflow.type !== 'default' ? { workflow: { ...conversation.workflow } } : {}),
      files: files.map((file) =>
        isComposerFile(file)
          ? {
              kind: 'file',
              id: file.id,
              name: file.name,
              file: file.file ? toRaw(file.file) : null,
              upload: file.upload ? { ...file.upload } : null,
              ...(file.claim
                ? { claim: { file: { ...file.claim.file }, picture: file.claim.picture ? toRaw(file.claim.picture) : null } }
                : {}),
              ...(file.snippet ? { snippet: file.snippet } : {}),
            }
          : { ...file },
      ),
      scroll: conversation.scroll
        ? structuredClone(toRaw(conversation.scroll))
        : null,
    }
  }

  /**
   * Saves the drafts this page changed since it last saved or restored them.
   * Other tabs of this web browser save to the same storage, so a draft this
   * page did not change may be another tab's newer one; writing this page's
   * copy of it would replace that.
   */
  function saveDrafts(): void {
    // What a composer holds is its draft's change from the first keystroke,
    // restored from this web browser's storage or not.
    for (const item of items.value) {
      if (item.persistence === 'synced') {
        recordSync.changed(item)
      }
    }
    const userId = session.user?.id ?? product.snapshot?.user.id
    if (!userId) {
      return
    }
    const changed = items.value
      .filter((item) => restored.has(item.id))
      .map((item) => ({
        id: item.id,
        draft: persisted(item),
      }))
      .filter((entry) => changedDraft(entry.id, entry.draft))
    const current = lifetime
    // IndexedDB serializes readwrite transactions; issue them now, including on pagehide.
    for (const entry of changed) {
      const operation =
        entry.draft.local?.phase === 'draft' &&
        !entry.draft.pendingSends.length &&
        !entry.draft.text.trim() &&
        !entry.draft.files.length
          ? deleteDraft(userId, entry.id)
          : writeDraft(userId, entry.id, entry.draft)
      void operation.catch((error) => {
        // A draft that did not reach storage counts as changed, so the next
        // save tries it again.
        savedDrafts.delete(entry.id)
        if (current === lifetime) {
          storageError(error, 'save')
        }
      })
    }
  }

  /**
   * Writes, as the page closes, each composer's text the backend does not
   * have yet where the web browser writes it at once; the IndexedDB writes
   * still under way may be lost with the page.
   */
  function keepLastWords(): void {
    const userId = session.user?.id
    if (!userId) {
      return
    }
    for (const item of items.value) {
      if (restored.has(item.id) && (item.persistence !== 'synced' || recordSync.unsaved(item))) {
        writeLastWords(userId, item.id, {
          base: item.persistence === 'synced' ? recordSync.base(item) : null,
          text: item.draft,
          attachmentIds: [...item.attachmentIds],
        })
      }
    }
  }

  /**
   * Whether `draft` differs from what this page last saved or restored for
   * the conversation; it becomes what the page saved. A file is compared by
   * its id, name and upload, as the draft names it.
   */
  function changedDraft(id: string, draft: SavedDraft): boolean {
    const shape = JSON.stringify(draft)
    if (savedDrafts.get(id) === shape) {
      return false
    }
    savedDrafts.set(id, shape)
    return true
  }

  async function restoreLocalDrafts(): Promise<void> {
    const userId = session.user?.id
    if (!userId) {
      return
    }
    const signal = lifetime.signal
    try {
      const { drafts, unreadable } = await readLocalDrafts(userId)
      signal.throwIfAborted()
      for (const record of unreadable) {
        storageError(record, 'restore')
      }
      for (const draft of drafts) {
        if (
          !draft.local ||
          items.value.some((item) => item.id === draft.local?.conversation.id)
        ) {
          continue
        }
        if (
          draft.local.phase === 'draft' &&
          !draft.text.trim() &&
          !draft.files.length
        ) {
          continue
        }
        const conversation = newConversation({
          ...draft.local.conversation,
          model: null,
          automation: null,
          pluginsChanged: false,
          onlyWaitsForHost: false,
          draftRevision: 0,
          saved: 0,
          savedRevision: 0,
          panelRevision: 0,
          lastAttachment: 0,
          hostsRevision: 0,
          pluginRevisions: [],
          permissionRequests: 0,
          unreadPermissionRequests: 0,
          permissionsRevision: 0,
          sideQuestionsRevision: 0,
          shares: [],
          status: 'idle',
          lastTurn: null,
          contextVersion: 0,
          revision: 0,
          readRevision: 0,
          unread: false,
          workflow: { type: 'default' },
        })
        conversation.persistence = draft.local.phase
        conversation.load = 'ready'
        // Drafts lead the list, newest first, whether the channel's state
        // is here yet or not (`conversations.md` § Conversations and projects).
        const index = items.value.findIndex((item) =>
          item.persistence === 'synced' || item.createdAt < conversation.createdAt)
        items.value.splice(index < 0 ? items.value.length : index, 0, conversation)
        const restoredConversation = items.value.find(
          (item) => item.id === conversation.id,
        )!
        await restoreDraft(restoredConversation, signal, draft)
      }
    } catch (error) {
      if (!signal.aborted) {
        storageError(error, 'restore')
      }
    }
  }

  async function initialize(): Promise<void> {
    const signal = lifetime.signal
    restoringLocalDrafts.value = true
    await restoreLocalDrafts()
    if (signal.aborted) {
      return
    }
    restoringLocalDrafts.value = false
    product.start()
    // The address may name one of them, shown before they were back.
    openListed()
  }

  /**
   * Opens the conversation the address names once the list has it, unless
   * its opening failed, which waits for Retry.
   */
  function openListed(): void {
    const active = items.value.find((item) => item.id === product.activeConversationId)
    if (active && active.load !== 'failed' && !cache.get(active.id)) {
      void activate(active.id)
    }
  }

  watch(
    () =>
      items.value.map((item) => ({
        id: item.id,
        draft: persisted(item),
      })),
    saveDrafts,
    { flush: 'sync' },
  )

  async function restoreDraft(
    conversation: Conversation,
    signal: AbortSignal,
    saved?: SavedDraft,
  ): Promise<void> {
    if (restored.has(conversation.id) || !session.user) {
      return
    }
    try {
      const draft = saved ?? (await readDraft(session.user.id, conversation.id))
      signal.throwIfAborted()
      if (draft) {
        // A delivery the page did not see end is a message to try again,
        // which Retry sends as a message.
        conversation.pendingSends = draft.pendingSends.map((pending) => ({
          ...pending,
          way: 'send',
          error: pending.error || t('drafts.deliveryInterrupted'),
        }))
        conversation.messageEdit = restoreMessageEdit(draft.messageEdit ?? null)
        // A draft holds model settings only for a conversation without a
        // record.
        if (draft.model && conversation.persistence !== 'synced') {
          conversation.model = draft.model
        }
        if (draft.workflow && conversation.persistence !== 'synced') {
          conversation.workflow = draft.workflow
        }
        conversation.scroll = draft.scroll
        // The composer's text is here only when the backend does not have
        // it; a synced conversation's composer shows the backend's draft
        // otherwise, once read.
        recordSync.restore(conversation, draft.base)
        conversation.notSaved = draft.notSaved
        // A message on its way from the list keeps it out of the list.
        for (const pending of conversation.pendingSends) {
          if (pending.saved) {
            recordSync.hideSent(conversation, pending.saved, pending.id)
          }
        }
        conversation.draft = draft.text
        conversation.comments = draft.comments
        conversation.files = draft.files.map(savedAttachment)
        // The files of the message, in the order of its capsules; the rest
        // are those of the sends not yet accepted.
        const sending = sendingFileIds(conversation)
        conversation.attachmentIds = conversation.files
          .filter((file) => !sending.has(file.id))
          .map((file) => file.id)
      }
      // What a page of the conversation held as it closed is newer than the
      // record, whose last writes may not have made it.
      const words = takeLastWords(session.user.id, conversation.id)
      if (words) {
        recordSync.restore(conversation, words.base)
        conversation.draft = words.text
        conversation.attachmentIds = words.attachmentIds.filter((id) =>
          conversation.files.some((file) => file.id === id),
        )
      }
    } catch (error) {
      signal.throwIfAborted()
      storageError(error, 'restore')
    }
    restored.add(conversation.id)
    // The page starts from what storage held: saving the restored draft
    // unchanged could only replace a newer one another tab saved since.
    changedDraft(conversation.id, persisted(conversation))
    for (const file of conversation.files) {
      if (isComposerFile(file) && !file.upload) {
        // A pasted file whose attaching the page did not see end attaches
        // again, and falls back as a paste does (`composer.md` § Writing a message).
        // Any other failure its capsule shows, with Retry.
        const claimed = !!file.claim
        void uploadInComposer(file).then((uploaded) => {
          if (!uploaded && claimed && !file.file) {
            unattached(conversation, file)
          }
        })
      }
    }
  }

  /**
   * A pasted file the paste could not use, with no picture to upload in its
   * place: its name stands where its capsule stood in the composer, and a
   * toast says so. A file of a message sent meanwhile stays; its bubble says
   * it was not delivered.
   */
  function unattached(conversation: Conversation, file: ProductAttachment): void {
    const index = conversation.attachmentIds.indexOf(file.id)
    if (index < 0) {
      return
    }
    const doc = parseUserMarkdown(conversation.draft, conversation.attachmentIds.map((id) => ({ id })))
    for (const block of doc.content ?? []) {
      block.content = block.content?.map((node) =>
        node.type === 'attachment' && node.attrs?.['capsule']?.id === file.id ? { type: 'text', text: file.name } : node)
    }
    conversation.draft = serializeUserMarkdown(doc).markdown
    arrangeFiles(conversation, conversation.attachmentIds.filter((id) => id !== file.id))
    conversation.draftShown += 1
    const toast = missingFilesToast([file.name])
    if (toast) {
      showToast({ ...toast, tone: 'danger' })
    }
  }

  /** A file of a draft this web browser kept, as the composer carries it. */
  function savedAttachment(file: SavedFile | Extract<ProductAttachment, { kind: 'reference' }>): ProductAttachment {
    if (file.kind === 'reference') {
      return file
    }
    return {
      ...(file.file
        ? composerAttachmentFromFile(file.file)
        : { src: file.upload ? blobPicture(file.upload) : file.claim ? blobPicture(file.claim.file) : undefined }),
      ...file,
      phase: file.upload ? 'ready' : file.claim ? 'attaching' : 'uploading',
    }
  }

  /**
   * A file of a draft the backend holds, as the composer carries it: the one
   * the composer already has for the same upload or remote file, or a new
   * one known by its upload alone.
   */
  function draftAttachment(conversation: Conversation, file: DraftFile): ProductAttachment {
    if (file.type === 'upload') {
      const carried = conversation.files.find(
        (item) => isComposerFile(item) && item.upload?.id === file.ref,
      )
      const upload = { id: file.ref, mediaType: file.mediaType, sha256: file.sha256 }
      return carried ?? {
        kind: 'file',
        id: file.ref,
        name: file.fileName,
        src: blobPicture(upload),
        phase: 'ready',
        ...(file.snippet ? { snippet: file.snippet } : {}),
        file: null,
        upload,
      }
    }
    const carried = conversation.files.find(
      (item) => item.kind === 'reference' && item.deviceId === file.deviceId && item.path === file.path,
    )
    const host =
      conversation.attachedHosts.shown.value?.find((attached) => attached.deviceId === file.deviceId)?.name ??
      resources.deviceById(file.deviceId)?.name ??
      file.deviceId
    return carried ?? { ...composerRemoteAttachment({ host, path: file.path }), deviceId: file.deviceId }
  }

  /** Shows `writing` in the conversation's composer, from outside: another page's draft, a restored one, or what the slot shows. */
  function showWriting(conversation: Conversation, writing: Writing): void {
    const files = writing.files.map((file) => draftAttachment(conversation, file))
    conversation.draft = writing.text
    conversation.comments = [...writing.comments]
    uploads.showFiles(conversation, files)
    conversation.draftShown += 1
  }

  /**
   * Changes the comments waiting in the conversation's draft
   * (`comments-and-ask.md` § Waiting comments), which the draft saves at
   * once, a change made in one step rather than typed.
   */
  function setComments(conversation: Conversation, comments: MessageComment[]): void {
    if (conversation.archived) {
      return
    }
    conversation.comments = comments
    saveDrafts()
    void recordSync.save(conversation).catch((error) => report(t('failures.saveDraft'), error))
  }

  /** What the replies' comments do in the conversation's draft (`ReplyCommenting`). */
  function commenting(conversation: Conversation): ReplyCommenting {
    return {
      get waiting() {
        return conversation.comments
      },
      add: (comment) => setComments(conversation, [...conversation.comments, comment]),
      sendNow: (comment) => void sendComment(conversation, comment),
      edit: (id, text) => setComments(conversation, conversation.comments.map((each) => (each.id === id ? { ...each, text } : each))),
      remove: (id) => setComments(conversation, conversation.comments.filter((each) => each.id !== id)),
    }
  }

  /** × on the composer's pill: every waiting comment goes. */
  function clearComments(conversation: Conversation): void {
    setComments(conversation, [])
  }


  /** Where a conversation's edit is kept while it is sent, and the session it is sent to. */
  function editHost(conversation: Conversation): MessageEditHost {
    return {
      get: () => conversation.messageEdit,
      set: (state) => { conversation.messageEdit = state },
      send: async (request, regenerate) => {
        if (regenerate) {
          const { operationId, targetBlockId, version } = request
          await (await runtimeFor(conversation)).regenerate({ operationId, targetBlockId, version })
          return
        }
        const edit = sentEditRequest(request)
        const runtime = await runtimeFor(conversation)
        await runtime.editAndSend(edit)
      },
    }
  }

  /**
   * Shows the conversation `id`, or none. `newConversation`: the address is
   * a new conversation's, as its history entry says, so it has no record to
   * read before its first send.
   */
  async function activate(id: string | null, options: { newConversation?: boolean } = {}): Promise<void> {
    const previous = items.value.find(
      (item) => item.id === product.activeConversationId,
    )
    if (previous && previous.id !== id && isEmptyDraft(previous)) {
      saveDrafts()
      items.value = items.value.filter((item) => item !== previous)
      restored.delete(previous.id)
      savedDrafts.delete(previous.id)
      cache.delete(previous.id)
    }
    product.activeConversationId = id
    const conversation = items.value.find((item) => item.id === id)
    if (!conversation) {
      // The first load opens the conversation its address names by its id,
      // before the channel's first state names it (`web-application.md`
      // § Requests for one action), but not a new conversation's, which has
      // no record before its first send.
      if (id && !options.newConversation && !product.snapshot && !earlyReads.has(id)) {
        earlyReads.set(id, openingReads(id, lifetime.signal))
      }
      return
    }
    if (conversation.persistence === 'synced') {
      copies.opened(conversation.id)
    }
    // A conversation the page shows already stays shown while it opens: the
    // first send opens the record it just created, and the composer the user
    // sent from must stay where it is, with the focus in it.
    if (!cache.get(conversation.id) && conversation.load !== 'ready') {
      if (conversation.persistence === 'synced') {
        startLoading(conversation)
      } else {
        conversation.load = 'ready'
      }
    }
    try {
      await cache.open(conversation.id, (entry) => loadConversation(conversation, entry))
      // Navigation retries a connection whose opening failed, without
      // refetching REST history; an open one is kept as it is, and an
      // archived one has none.
      if (!conversation.archived) {
        await cache.get(conversation.id)?.runtime?.connect()
      }
    } catch {
      // The failure is the conversation's load state: the pane or the
      // transcript's tail notice tells it, with Retry.
    }
  }

  async function reloadSession(id: string): Promise<void> {
    cache.delete(id)
    await activate(id)
  }

  /**
   * The reads an opening needs that name only the conversation: the latest
   * page of its transcript, its subagents, its attached hosts and its
   * draft, sent together (`web-application.md` § Requests for one action).
   */
  interface OpeningReads {
    transcript: Promise<TranscriptPage>
    /** Resolves once a try of the transcript's read could not reach the backend, and the read waits for it. */
    unreachable: Promise<void>
    subagents: Promise<Subagents>
    hosts: Promise<AttachedHosts>
    draft: Promise<ConversationDraft>
  }

  /** Reads the first load started for the conversation its address names, before the channel's first state. */
  const earlyReads = new Map<string, OpeningReads>()
  /** Conversations whose record this page just created, which their opening does not read back. */
  const madeHere = new Set<string>()

  function openingReads(id: string, signal: AbortSignal): OpeningReads {
    const path = `/conversations/${encodeURIComponent(id)}`
    const unreachable = Promise.withResolvers<void>()
    const reads = {
      transcript: apiRequest(`${path}/transcript`, { signal, onUnreachable: () => unreachable.resolve() })
        .then((response) => readResponse(response, transcriptPageSchema)),
      subagents: apiRequest(`${path}/subagents`, { signal }).then((response) => readResponse(response, subagentsSchema)),
      hosts: apiRequest(`${path}/hosts`, { signal }).then((response) => readResponse(response, attachedHostsSchema)),
      draft: loadDraft(id, signal),
    }
    // The opening that takes them reports their failures; one never taken has nobody to tell.
    for (const read of Object.values(reads)) {
      read.catch(() => {})
    }
    return { ...reads, unreachable: unreachable.promise }
  }

  /** The conversation's socket address. */
  function streamUrl(id: string): string {
    const url = new URL(`/api/conversations/${encodeURIComponent(id)}/stream`, window.location.href)
    url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:'
    return url.toString()
  }

  async function loadConversation(
    conversation: Conversation,
    entry: CachedConversation,
  ): Promise<void> {
    const { controller } = entry
    const { signal } = controller
    // An archive or a restore on its way lands first, so the conversation
    // opens as the backend then has it.
    const landing = archiving.get(conversation.id)
    if (landing) {
      await landing
      signal.throwIfAborted()
    }
    // The socket's connection is made beside the reads, for a conversation
    // whose record names a model, which is one that can open; its `open`
    // waits for the transcript, which the socket's frames continue.
    const runnable = conversation.persistence === 'synced' && !conversation.archived &&
      !!conversation.model.providerId && !!conversation.model.modelId
    let early: ReturnType<typeof connectConversationClient> | null = runnable ? connectConversationClient(streamUrl(conversation.id), signal) : null
    early?.catch(() => {})
    const discardEarly = () => {
      void early?.then((client) => client.disconnect(), () => {})
      early = null
    }
    try {
      const draftRestored = restoreDraft(conversation, signal)
      if (conversation.persistence !== 'synced') {
        await draftRestored
        conversation.load = 'ready'
        return
      }
      // Share global model discovery, but render history before it completes.
      // Model load errors belong to the composer, not transcript restoration.
      void product.loadModels().catch(() => {})
      let hostsRead: Promise<void> = Promise.resolve()
      if (madeHere.delete(conversation.id)) {
        // Its record is what the page created: no transcript yet, the hosts
        // its answer listed, and the composer's draft.
        await draftRestored
        conversation.load = 'ready'
      } else {
        hostsRead = (await readOpening(conversation, signal, draftRestored, runnable)).hosts
      }
      // The session starts with a model of the catalog that answers for the
      // providers as they are now: a change of them while the conversation
      // opens, which empties the catalog or replaces its load, is waited
      // for, so a message sent meanwhile goes once it has answered.
      await product.catalogAnswered(signal)
      signal.throwIfAborted()
      const made = early
      early = null
      await startSession(conversation, entry, made)
      // No session started: archived meanwhile, or its model is not one the
      // page can use.
      if (!entry.runtime && conversation.load === 'opening') {
        conversation.load = 'ready'
      }
      await hostsRead
    } catch (error) {
      discardEarly()
      if (!signal.aborted) {
        failOpening(conversation, error)
      }
      throw error
    }
  }

  /**
   * The conversation did not open: the pane or the transcript's tail says
   * why, with Retry. A database the server could not read is said in the
   * page's words; the error itself goes to the console.
   */
  function failOpening(conversation: Conversation, error: unknown): void {
    console.error('The conversation did not open', error)
    conversation.load = 'failed'
    // A refusal is worded from its code, as `conversation_unreadable` is in
    // the page's words rather than the server's, which name its files.
    conversation.lastError = shownError(error)
  }

  /**
   * Starts the session of a conversation whose transcript the page holds,
   * with a model it can use and unless it is archived, and connects it;
   * `made` is a connection made beside the opening's reads, which the first
   * attempt takes.
   */
  async function startSession(
    conversation: Conversation,
    entry: CachedConversation,
    made: ReturnType<typeof connectConversationClient> | null,
  ): Promise<void> {
    const { signal } = entry.controller
    const pick = composerModel(
      resources.providerInfos,
      resources.models,
      conversation.model.providerId,
      conversation.model.modelId,
    )
    if (conversation.archived || pick.kind !== 'ready') {
      void made?.then((client) => client.disconnect(), () => {})
      return
    }
    let first = made
    const runtime = new ConversationRuntime({
      state: conversation,
      connect: async (attempt) => {
        const taken = first
        first = null
        if (taken) {
          return takeConnection(taken, attempt)
        }
        // A restore on its way lands first: the backend refuses an archived
        // conversation's socket.
        await archiving.get(conversation.id)
        attempt.throwIfAborted()
        return connectConversationClient(streamUrl(conversation.id), attempt)
      },
      shownSubagents: () => conversation.shownSubagents,
      onEvent: (next) => {
        applyConversationEvent(conversation, next)
        reconcileSubmission(conversation)
        if (next.type === 'shell_output') {
          const terminal = conversation.terminals.find((candidate) => candidate.id === next.status.commandId)
          if (terminal) {
            void readCommandCall(conversation, terminal)
          }
        }
      },
    })
    entry.runtime = runtime
    try {
      await runtime.connect()
    } catch (error) {
      // Archived meanwhile, the session waits for Restore with all it shows.
      if (conversation.archived && !signal.aborted) {
        return
      }
      // A record that holds no model, such as one an automation's run
      // started, opens no session until one is chosen: it shows its
      // transcript, and the composer's choice writes the record and opens
      // it (`changeModel`), never a failed opening.
      if (errorCode(error) === 'model_not_selected' && !signal.aborted) {
        entry.runtime = null
        runtime.dispose()
        return
      }
      throw error
    }
  }

  /**
   * Shows the conversation loading, as an opening begins: nothing of an
   * earlier attempt shows meanwhile, neither its transcript and agents nor
   * its session or its failure (`web-application.md` § Opening a
   * conversation shows in two steps).
   */
  function startLoading(conversation: Conversation): void {
    conversation.load = 'loading'
    conversation.history = EMPTY_TRANSCRIPT
    conversation.shownAt = null
    conversation.failures = {}
    conversation.instructions = []
    conversation.summaries = {}
    conversation.subagents = []
    conversation.lastError = null
    clearSession(conversation)
  }

  /**
   * Clears what only the live session knows, which joins at the second
   * step of the opening; nothing of a session the page held before shows
   * meanwhile.
   */
  function clearSession(conversation: Conversation): void {
    conversation.phase = 'idle'
    conversation.queue = []
    conversation.pendingSteers = []
    conversation.pendingCalls = []
    conversation.waitingSince = null
    conversation.contextUsage = null
    conversation.terminals = []
  }

  /**
   * Reads what the page shows of a conversation it opens, all at once: its
   * transcript, its agents, its hosts and its draft, and shows the first
   * step of its opening, the transcript, once it and the draft are read
   * (`web-application.md` § Opening a conversation shows in two steps):
   * `runnable`, its session opens next, which the second step shows;
   * otherwise it stops there. The agents and the hosts join when they
   * arrive; it answers once the transcript is held, with the hosts' read,
   * which neither the transcript nor the session's opening waits for.
   */
  async function readOpening(
    conversation: Conversation,
    signal: AbortSignal,
    draftRestored: Promise<void>,
    runnable: boolean,
  ): Promise<{ hosts: Promise<void> }> {
    const reads = earlyReads.get(conversation.id) ?? openingReads(conversation.id, signal)
    earlyReads.delete(conversation.id)
    const hostsRevision = conversation.attachedHosts.requested()
    const hostsSince = conversation.attachedHosts.now()
    // Taken after this web browser's own copy; a submission it confirms
    // clears the draft after it, so the transcript waits for it.
    // The reads count as those of the revisions the summary names now.
    const summary = product.snapshot?.conversations.find((record) => record.id === conversation.id)
    conversation.record.ask(summary?.draftRevision ?? 0, 'draft')
    conversation.record.ask(summary?.savedRevision ?? 0, 'saved')
    const draftRead = draftRestored.then(() => recordSync.readDraft(conversation, reads.draft))
    // The composer's entry shows the list once read; nothing waits for it.
    void recordSync.readSaved(conversation).catch((error) => report(t('failures.readSaved'), error))
    const hostsRead = reads.hosts.then((answer) => {
      signal.throwIfAborted()
      // A summary that raised the revision meanwhile started a newer read,
      // which this one then gives way to.
      conversation.attachedHosts.replace(answer.hosts, { revisions: { '': hostsRevision }, since: hostsSince })
    })
    hostsRead.catch(() => {})
    // A transcript that cannot be read shows the copy the page kept for
    // opening offline meanwhile; once the backend answers, the conversation
    // opens from it below, as if it had never shown the copy.
    let answered = false
    void reads.unreachable.then(() => openFromCopy(conversation, signal, () => answered))
    const [page] = await Promise.all([reads.transcript, draftRead])
    answered = true
    signal.throwIfAborted()
    // The latest page is the transcript's end until the stream says more.
    conversation.history = addPage(EMPTY_TRANSCRIPT, page, false)
    conversation.shownAt = null
    conversation.failures = page.failures ?? {}
    conversation.instructions = page.instructions
    conversation.summaries = Object.fromEntries(page.summaries.map((summary) => [summary.marker, summary.summaryTokens]))
    reconcileSubmission(conversation)
    clearSession(conversation)
    // The first copy of the children, which joins when it arrives and which
    // the open's `subagents` frame replaces whole (`subagents.md`
    // § Protocol); while the tree is live the read is the tree's own list,
    // so the frame takes back none of them. A read that arrives once the
    // session has opened is older than its frame, and gives way to it, its
    // failure too: the frame listed the children.
    const sessionOpened = () => cache.get(conversation.id)?.runtime?.hasOpened ?? false
    void reads.subagents.then(
      (listed) => {
        if (!signal.aborted && !sessionOpened()) {
          conversation.subagents = listed.subagents.map(subagentRecord)
        }
      },
      (error: unknown) => {
        if (!signal.aborted && !sessionOpened()) {
          report(t('failures.readAgents'), error)
        }
      },
    )
    // The status stays the summary's until the session tells its phase.
    conversation.load = runnable ? 'opening' : 'ready'
    return { hosts: hostsRead }
  }

  /**
   * Takes `ids` as the children whose transcripts the page shows, and tells
   * the session. A running child it stops showing streams no more, so the
   * page lets go of its transcript, which would go stale, and reads or
   * receives it again when it shows the child again; a finished one keeps it.
   */
  function showSubagents(conversation: Conversation, ids: string[]): void {
    const dropped = conversation.shownSubagents.filter((id) => !ids.includes(id))
    if (dropped.length === 0 && ids.every((id) => conversation.shownSubagents.includes(id))) {
      return
    }
    conversation.shownSubagents = ids
    for (const agent of conversation.subagents) {
      if (dropped.includes(agent.id) && agent.phase === 'running') {
        agent.history = EMPTY_TRANSCRIPT
        agent.failures = {}
      }
    }
    cache.get(conversation.id)?.runtime?.showSubagents()
  }

  /** A child as the page holds it before its session says more, from its job. */
  function subagentRecord(job: AgentCopy): Conversation['subagents'][number] {
    return {
      ...subagentJobFields(job),
      pendingCalls: [],
      waitingSince: null,
      // A live child's `phase` frame comes with the opening handshake.
      sessionPhase: 'idle',
      history: EMPTY_TRANSCRIPT,
      failures: {},
      // A live child's usage comes with the opening handshake.
      contextUsage: null,
    }
  }

  /**
   * Shows the conversation from the copy the page kept for opening offline,
   * as the first step of an opening that starts no session; one with no
   * copy says it is not available offline. Nothing changes once the
   * backend `answered` or the opening ended meanwhile.
   */
  async function openFromCopy(conversation: Conversation, signal: AbortSignal, answered: () => boolean): Promise<void> {
    const userId = session.user?.id
    const copy = userId ? await readConversationCopy(userId, conversation.id) : null
    if (signal.aborted || answered() || conversation.load !== 'loading') {
      return
    }
    if (!copy) {
      conversation.load = 'offline'
      return
    }
    const { transcript } = copy
    conversation.history = heldTranscript(
      transcript.length,
      transcript.blocks.length > 0 ? [{ start: transcript.start, blocks: transcript.blocks }] : [],
    )
    conversation.shownAt = null
    conversation.failures = copy.failures
    conversation.instructions = copy.instructions
    conversation.summaries = copy.summaries
    conversation.subagents = copy.agents.map(subagentRecord)
    clearSession(conversation)
    conversation.load = 'ready'
  }

  /**
   * Reads the hosts of an open conversation again, after a summary raised
   * their revision (`web-api-conversations.md` § Sidebar mutations and read state); an
   * answer that a later read overtook is dropped.
   */
  async function readHosts(conversation: Conversation): Promise<void> {
    const revision = conversation.attachedHosts.requested()
    const since = conversation.attachedHosts.now()
    const response = await apiRequest(`/conversations/${encodeURIComponent(conversation.id)}/hosts`, {
      signal: lifetime.signal,
    })
    const answer = await readResponse(response, attachedHostsSchema)
    conversation.attachedHosts.replace(answer.hosts, { revisions: { '': revision }, since })
  }

  async function runtimeFor(
    conversation: Conversation,
  ): Promise<ConversationRuntime> {
    if (conversation.archived) {
      throw coded(new Error('Restore this conversation before sending.'), 'restore_before_sending')
    }
    const cached = cache.get(conversation.id)
    if (cached) {
      await cached.opening
      if (!cached.runtime) {
        cache.delete(conversation.id)
      }
    }
    await activate(conversation.id)
    // A change of the conversation's context, which the channel can bring
    // while the conversation opens, replaces its entry: the send waits for
    // the entry that stands.
    for (let entry = cache.get(conversation.id); entry && !entry.runtime;) {
      await entry.opening.catch(() => {})
      const standing = cache.get(conversation.id)
      if (standing === entry) {
        break
      }
      entry = standing
    }
    const runtime = cache.get(conversation.id)?.runtime
    if (!runtime) {
      // A model list that failed is why no model could be picked
      // (`web-application.md` § Model discovery).
      const failure = product.catalogFailure
      throw conversation.lastError
        ? new Error(conversation.lastError.message)
        : failure === null
          ? coded(new Error('No available model for this conversation.'), 'no_model_available')
          : coded(new Error(`Couldn’t load the models: ${failure}`), 'models_not_loaded', { failure })
    }
    return runtime
  }

  /**
   * The patch that writes the whole of `settings` to a record. A model that
   * lists no efforts has none to name; the backend chooses the first effort
   * of one whose settings name none.
   */
  function settingsPatch(settings: ModelSettings): ConversationPatch {
    return {
      model: { providerId: settings.providerId, modelId: settings.modelId },
      ...(settings.thinkingEffort === null ? {} : { thinkingEffort: settings.thinkingEffort }),
      serviceTierId: settings.serviceTierId,
    }
  }

  /**
   * Sends `changes` of the conversation `id` and applies the record it
   * answers; with `shown`, the change the page shows meanwhile, which lands
   * with it, or leaves when a field was refused. Reports refused fields and
   * answers whether every field applied.
   */
  async function patch(id: string, changes: ConversationPatch, shown?: ProductChange): Promise<boolean> {
    const signal = lifetime.signal
    const sentAt = product.sent()
    shown?.send()
    const response = await apiRequest(
      `/conversations/${encodeURIComponent(id)}`,
      {
        method: 'PATCH',
        signal: lifetime.signal,
        ...jsonBody(changes),
      },
    )
    const result = await readResponse(response, conversationUpdateSchema)
    signal.throwIfAborted()
    const failed = result.results.flatMap((field) => field.status === 'failed' ? [field] : [])
    // The answer is the record as the patch left it, which shows at once.
    const answer = { type: 'conversation', conversation: result.conversation } as const
    if (!shown) {
      product.answered(sentAt, answer)
    } else if (failed.length) {
      shown.drop(answer)
    } else {
      shown.land(answer)
    }
    if (failed.length) {
      reportError(
        t('failures.fieldsNotUpdated'),
        failed.map((field) => `${field.field}: ${field.message}`).join('\n'),
        { userVisible: true, expected: true },
      )
    }
    return failed.length === 0
  }

  /** A move into a project shows its directory, where the conversation will run. */
  function shownDirectory(
    state: ProductState,
    target: ConversationSummary['target'] | undefined,
    record: ConversationSummary,
  ): { cwd: string } {
    const path = target?.kind === 'workspace'
      ? state.workspaces.find((workspace) => workspace.id === target.workspaceId)?.path
      : undefined
    return { cwd: path ?? record.cwd }
  }

  async function changeConversations<T>(
    ids: string[],
    action: () => Promise<T>,
    busyResult: T,
  ): Promise<T> {
    if (ids.some((id) => pendingChanges.value.includes(id))) {
      return busyResult
    }
    const current = lifetime
    pendingChanges.value.push(...ids)
    try {
      return await action()
    } finally {
      if (lifetime === current) {
        pendingChanges.value = pendingChanges.value.filter(
          (id) => !ids.includes(id),
        )
      }
    }
  }

  /**
   * Changes the conversations' `changes`: a draft's at once, and those with
   * a record by a write, after the earlier writes of each of them. With
   * `shown`, the change shows at once and the write lands it
   * (`web-application.md` § Responding to the user); without, it shows once
   * answered, as a move to another Host does. A refused item is reported
   * with why, in a toast that says it could not `action` it, and only its
   * own change leaves. Answers whether every one changed, and
   * the ones that did.
   */
  async function applyBatch(
    ids: string[],
    changes: Pick<ConversationPatch, 'pinned' | 'archived' | 'target' | 'notifyAgent'>,
    shown: boolean,
    action: RefusedAction,
  ): Promise<{ success: boolean; changed: string[] }> {
    // A draft has no agent to tell: the first send creates what the user sees.
    const { notifyAgent: _told, ...fields } = changes
    for (const conversation of items.value) {
      if (
        ids.includes(conversation.id) &&
        conversation.persistence !== 'synced'
      ) {
        Object.assign(conversation, fields)
        conversation.projectId =
          conversation.target.kind === 'workspace'
            ? conversation.target.workspaceId
            : null
      }
    }
    const changed = ids.filter((id) => items.value.some((item) => item.id === id && item.persistence !== 'synced'))
    ids = ids.filter(
      (id) =>
        items.value.find((item) => item.id === id)?.persistence === 'synced',
    )
    saveDrafts()
    if (!ids.length) {
      return { success: true, changed }
    }
    // A conversation opened before the archive or restore lands waits for
    // it; the entry stands before the change shows, which may open it.
    const landed = deferred<void>()
    if (fields.archived !== undefined) {
      for (const id of ids) {
        archiving.set(id, landed.promise)
      }
    }
    const shownChanges = new Map(shown
      ? ids.map((id) => [id, product.change(`conversation:${id}`, (state) =>
          withConversation(state, id, (record) => ({ ...record, ...fields, ...shownDirectory(state, fields.target, record) })))])
      : [])
    const signal = lifetime.signal
    try {
      return await writes.run(ids, async () => {
        signal.throwIfAborted()
        let success = true
        for (let offset = 0; offset < ids.length; offset += 100) {
          const chunk = ids.slice(offset, offset + 100)
          const sentAt = product.sent()
          for (const id of chunk) {
            shownChanges.get(id)?.send()
          }
          const response = await apiRequest('/conversations/batch', {
            method: 'POST',
            signal,
            ...jsonBody({
              items: chunk.map((id) => ({ id, patch: changes })),
            } satisfies ConversationBatch),
          })
          const result = await readResponse(response, batchAnswerSchema)
          signal.throwIfAborted()
          const requestId = response.headers.get(REQUEST_ID_HEADER)
          const refusals: Refusal[] = []
          for (const item of result.results) {
            const change = shownChanges.get(item.id)
            // A refusal names the conversation as the list shows it now: its title may have come meanwhile.
            const listed = items.value.find((conversation) => conversation.id === item.id)
            const title = listed ? conversationTitle(listed.title) : item.id
            if (item.status === 'refused') {
              change?.drop()
              // A conversation another page deleted meanwhile has nothing
              // left to change, and the channel takes its row away.
              if (item.code !== 'conversation_not_found') {
                refusals.push({ title, reasons: [item.message], requestId })
              }
              continue
            }
            const answer = { type: 'conversation', conversation: item.conversation } as const
            const failed = item.results.flatMap((field) => field.status === 'failed' ? [field] : [])
            if (!change) {
              product.answered(sentAt, answer)
            } else if (failed.length) {
              change.drop(answer)
            } else {
              change.land(answer)
            }
            if (failed.length) {
              refusals.push({ title, reasons: failed.map((field) => field.message), requestId })
            } else {
              changed.push(item.id)
            }
          }
          if (refusals.length) {
            success = false
            showRefused(action, refusals)
          }
        }
        return { success, changed }
      })
    } catch (error) {
      report(t('failures.updateConversations'), error)
      return { success: false, changed }
    } finally {
      // A change the answer did not name, or one whose write failed, leaves.
      for (const change of shownChanges.values()) {
        change.drop()
      }
      landed.resolve()
      for (const id of ids) {
        if (archiving.get(id) === landed.promise) {
          archiving.delete(id)
        }
      }
    }
  }

  /**
   * Archives the conversations, which leave the list at once, and says so in
   * a toast whose Undo restores them (`conversations.md` § Conversations and
   * projects). Once the backend answered, the toast counts, and its Undo
   * restores, only those it archived; the refusal's own toast says why of
   * the others. Answers whether every one was archived.
   */
  async function archive(ids: string[]): Promise<boolean> {
    const shown = ids.filter((id) => items.value.some((item) => item.id === id && !item.archived))
    const done = applyBatch(ids, { archived: true }, true, 'archive')
    if (!shown.length) {
      return (await done).success
    }
    const toast = showArchived(shown.length, () => void restore(shown))
    const { success, changed } = await done
    const archived = shown.filter((id) => changed.includes(id))
    if (!archived.length) {
      dismissToast(toast)
    } else if (archived.length < shown.length) {
      updateArchived(toast, archived.length, () => void restore(archived))
    }
    return success
  }

  /**
   * Deletes the conversations for good, once the user confirmed it
   * (`conversations.md` § Conversations and projects): they leave the list at
   * once; one the backend has no record of leaves this page, and each other
   * one leaves the backend and every page. What this browser keeps of their
   * drafts goes too. A refusal brings the conversation back with a toast.
   * Answers whether every one was deleted.
   */
  async function remove(ids: string[]): Promise<boolean> {
    const userId = session.user?.id ?? product.snapshot?.user.id
    const forget = (id: string) => {
      restored.delete(id)
      savedDrafts.delete(id)
      if (userId) {
        void deleteDraft(userId, id).catch((error) => storageError(error, 'save'))
      }
    }
    const synced = ids.filter((id) => items.value.find((item) => item.id === id)?.persistence === 'synced')
    const local = ids.filter((id) => !synced.includes(id))
    items.value = items.value.filter((item) => !local.includes(item.id))
    local.forEach(forget)
    const titles = new Map(synced.map((id) => {
      const listed = items.value.find((item) => item.id === id)
      return [id, listed ? conversationTitle(listed.title) : id]
    }))
    const removals = new Map(synced.map((id) => [id, product.change(`conversation:${id}`, (state) => ({
      ...state,
      conversations: state.conversations.filter((record) => record.id !== id),
    }))]))
    const signal = lifetime.signal
    try {
      return await writes.run(synced, async () => {
        const refusals: Refusal[] = []
        for (const [id, removal] of removals) {
          signal.throwIfAborted()
          removal.send()
          try {
            await apiRequest(`/conversations/${encodeURIComponent(id)}`, { method: 'DELETE', signal })
          } catch (error) {
            if (signal.aborted) {
              throw error
            }
            // One another page deleted meanwhile is deleted: that is done.
            if (errorCode(error) === 'conversation_not_found') {
              removal.land()
              forget(id)
              continue
            }
            removal.drop()
            const { message, requestId } = shownError(error)
            refusals.push({ title: titles.get(id) ?? id, reasons: [message], requestId })
            continue
          }
          // The deletion lands; the channel brings it to every other page.
          removal.land()
          forget(id)
        }
        if (refusals.length) {
          showRefused('delete', refusals)
        }
        return refusals.length === 0
      })
    } catch (error) {
      for (const removal of removals.values()) {
        removal.drop()
      }
      report(t('failures.deleteConversations'), error)
      return false
    }
  }

  /** Brings archived conversations back at once; answers whether every one came back. */
  async function restore(ids: string[]): Promise<boolean> {
    const { success } = await applyBatch(ids, { archived: false }, true, 'restore')
    return success
  }

  /**
   * Opens a new conversation in the project `projectId`, or outside
   * projects, with the choices last made there (`web-api-settings.md` § User
   * preferences); an empty one already open there takes them again.
   */
  function create(projectId: string | null = null): string {
    const starting = startingChoices(
      product.snapshot?.workspaces.find((workspace) => workspace.id === projectId) ?? null,
      { lastModel: preferences.lastModel, lastWorkflow: preferences.lastWorkflow },
    )
    const empty = items.value.find(
      (item) => isEmptyDraft(item) && !item.archived && item.projectId === projectId,
    )
    if (empty) {
      empty.model = initialModelSettings(starting.model)
      empty.workflow = starting.workflow
      saveDrafts()
      return empty.id
    }
    const now = new Date().toISOString()
    const conversation = newConversation({
      id: crypto.randomUUID(),
      title: null,
      pinned: false,
      archived: false,
      automation: null,
      target: projectId
        ? { kind: 'workspace', workspaceId: projectId }
        : { kind: 'cloud' },
      contextVersion: 0,
      revision: 0,
      readRevision: 0,
      unread: false,
      workflow: starting.workflow,
      pluginsChanged: false,
      onlyWaitsForHost: false,
      draftRevision: 0,
      saved: 0,
      savedRevision: 0,
      panelRevision: 0,
      lastAttachment: 0,
      hostsRevision: 0,
      pluginRevisions: [],
      permissionRequests: 0,
      unreadPermissionRequests: 0,
      permissionsRevision: 0,
      sideQuestionsRevision: 0,
      shares: [],
      createdAt: now,
      updatedAt: now,
      model: null,
      status: 'idle',
      lastTurn: null,
    })
    conversation.persistence = 'draft'
    conversation.model = initialModelSettings(starting.model)
    conversation.load = 'ready'
    restored.add(conversation.id)
    items.value.unshift(conversation)
    return conversation.id
  }

  async function fork(sourceId: string, request: MessageForkRequest): Promise<string> {
    const signal = lifetime.signal
    const sentAt = product.sent()
    const { conversation: record } = await forkConversation(sourceId, request, signal)
    signal.throwIfAborted()
    // The destination's record holds the model settings it inherited.
    product.answered(sentAt, { type: 'conversation', conversation: record })
    return record.id
  }

  /**
   * Creates the conversation's record with one request that carries its
   * target and its settings (`web-api-conversations.md` § Conversation creation and Fork):
   * for a part its model no longer offers, the model's first effort and the
   * vendor's default tier. The page shows what the answer says; a record
   * this request made is opened without reading it back.
   */
  async function persistConversation(
    conversation: Conversation,
  ): Promise<void> {
    if (conversation.persistence === 'synced') {
      return
    }
    const signal = lifetime.signal
    const sentAt = product.sent()
    const shown = shownSettings(conversation)
    const listed = lookupSelectedModel(resources.models, shown.providerId, shown.modelId)
    const settings = listed ? offeredSettings(shown, listed.model) : null
    const request: CreateConversation = {
      id: conversation.id,
      // The placeholder goes as no title, which holds no words.
      ...(conversation.title === null ? {} : { title: conversation.title }),
      pinned: conversation.pinned,
      target: conversation.target,
      ...(settings ? settingsPatch(settings) : {}),
      ...(conversation.workflow.type === 'default' ? {} : { workflow: conversation.workflow }),
    }
    const response = await apiRequest('/conversations', {
      method: 'POST',
      signal,
      ...jsonBody(request),
    })
    const result = await readResponse(response, createdConversationSchema)
    signal.throwIfAborted()
    product.answered(sentAt, { type: 'conversation', conversation: result.conversation })
    // The choice the menu made while the record was on its way, which the
    // request could not carry.
    const chosen = conversation.workflow
    conversation.persistence = 'synced'
    Object.assign(conversation, metadata(result.conversation))
    followRecordModel(conversation, result.conversation)
    conversation.attachedHosts.ask(result.conversation.hostsRevision)
    cache.delete(conversation.id)
    // A retry that found the record an earlier attempt made reads it.
    if (response.status === 201) {
      madeHere.add(conversation.id)
    }
    // It applies before the first message goes, so the first turn's spawns
    // and instructions follow it (`subagents.md` § Workflows of a project).
    if (!sameWorkflow(chosen, result.conversation.workflow)) {
      await applyWorkflow(conversation.id, chosen)
    }
  }

  function rename(id: string, title: string): void {
    const conversation = items.value.find((item) => item.id === id)
    if (conversation && conversation.persistence !== 'synced') {
      conversation.title = title
      saveDrafts()
      return
    }
    // The new title shows at once; a write that fails or is refused gives
    // the stored title back.
    const change = product.change(`conversation:${id}`, (state) =>
      withConversation(state, id, (record) => ({ ...record, title })))
    const signal = lifetime.signal
    void writes
      .run(id, () => {
        signal.throwIfAborted()
        return patch(id, { title }, change)
      })
      .catch((error) => {
        change.drop()
        // A conversation another page deleted meanwhile has no title to change.
        if (errorCode(error) !== 'conversation_not_found') {
          report(t('failures.renameConversation'), error)
        }
      })
  }

  /**
   * Moves the conversation before `beforeId`, or to the end of its part of
   * the list, at once; the write follows the earlier moves.
   */
  function reorder(id: string, beforeId: string | null): Promise<void> {
    const conversation = items.value.find((item) => item.id === id)
    // A new conversation before its first send moves among the others
    // like it, which keep the front; its first send gives it its place.
    if (conversation && conversation.persistence !== 'synced') {
      const reordered = items.value.filter((item) => item.id !== id)
      const before = reordered.find((item) => item.id === beforeId)
      const index = before && before.persistence !== 'synced'
        ? reordered.indexOf(before)
        : reordered.filter((item) => item.persistence !== 'synced').length
      reordered.splice(index, 0, conversation)
      items.value = reordered
      return Promise.resolve()
    }
    if (beforeId) {
      const index = items.value.findIndex((item) => item.id === beforeId)
      beforeId =
        items.value.slice(index).find((item) => item.persistence === 'synced')
          ?.id ?? null
    }
    const before = beforeId
    // The end of its part of the list is the end of the list there.
    const change = product.change('conversation_order', (state) => {
      const moved = state.conversations.find((record) => record.id === id)
      const next = state.conversations.find((record) => record.id === before) ?? null
      return moved && (before === null || next)
        ? { ...state, conversations: moveBefore(state.conversations, moved, next) }
        : state
    })
    const signal = lifetime.signal
    return writes
      .run('conversation_order', async () => {
        signal.throwIfAborted()
        change.send()
        await apiRequest('/sidebar/reorder', {
          method: 'POST',
          signal,
          ...jsonBody({
            kind: 'conversation',
            id,
            beforeId: before,
          } satisfies SidebarReorder),
        })
        change.land()
      })
      .catch((error) => {
        change.drop()
        // A move naming a conversation another page deleted meanwhile has
        // nothing to say; the channel brings the order there is.
        if (errorCode(error) !== 'conversation_not_found') {
          report(t('failures.reorderConversations'), error)
        }
      })
  }

  /**
   * Marks what the conversation shows as read, at once, once it shows its
   * transcript; the write follows the conversation's earlier ones.
   */
  function markRead(id: string): Promise<void> {
    const conversation = items.value.find((item) => item.id === id)
    if (
      !conversation ||
      !transcriptShown(conversation.load) ||
      !(conversation.unread || conversation.needsYou) ||
      document.visibilityState !== 'visible'
    ) {
      return Promise.resolve()
    }
    const revision = conversation.revision
    const change = product.change(`conversation:${id}`, (state) =>
      withConversation(state, id, (record) => {
        const readRevision = Math.max(record.readRevision, revision)
        return { ...record, readRevision, unread: record.revision > readRevision, unreadPermissionRequests: 0 }
      }))
    const signal = lifetime.signal
    return writes
      .run(id, async () => {
        signal.throwIfAborted()
        change.send()
        await apiRequest(`/conversations/${encodeURIComponent(id)}/read`, {
          method: 'POST',
          signal,
          ...jsonBody({ revision } satisfies ReadRequest),
        })
        change.land()
      })
      .catch((error) => {
        change.drop()
        // A conversation deleted meanwhile, by this page or another, has
        // nothing left to acknowledge.
        if (errorCode(error) === 'conversation_not_found') {
          return
        }
        report(t('failures.updateReadStatus'), error)
      })
  }

  /**
   * Opens the conversation's tree again with the plugins the user has on
   * (`web-api-sync.md` § A user's plugins); its socket is closed and reconnects,
   * and the summary stops offering the reload.
   */
  async function reloadPlugins(conversation: Conversation): Promise<void> {
    try {
      await apiRequest(
        `/conversations/${encodeURIComponent(conversation.id)}/reload`,
        { method: 'POST', signal: lifetime.signal },
      )
    } catch (error) {
      // A conversation another page deleted meanwhile has nothing to reload.
      if (errorCode(error) !== 'conversation_not_found') {
        report(t('failures.reloadConversation'), error)
      }
    }
  }

  /**
   * Detaches a device the conversation's agents attached; a draft has none.
   * It ends the device's access, so the host menu shows it in progress until
   * the backend answers (`web-application.md` § Responding to the user).
   */
  async function detachHost(
    conversation: Conversation,
    deviceId: string,
  ): Promise<void> {
    try {
      const sentAt = conversation.attachedHosts.now()
      await apiRequest(
        `/conversations/${encodeURIComponent(
          conversation.id,
        )}/hosts/${encodeURIComponent(deviceId)}`,
        {
          method: 'DELETE',
          signal: lifetime.signal,
        },
      )
      // The answer says the device is detached.
      conversation.attachedHosts.answered([''], sentAt, {
        value: (hosts) => hosts.filter((host) => host.deviceId !== deviceId),
      })
    } catch (error) {
      // A conversation another page deleted meanwhile has no device attached.
      if (errorCode(error) !== 'conversation_not_found') {
        report(t('failures.detachDevice'), error)
      }
    }
  }

  /**
   * Makes one change of the conversation's model settings, which names only
   * the parts the user changed (`web-api-conversations.md` § Sidebar mutations and read
   * state): a conversation with a record changes its record and shows the
   * answer, and every other page shows it from the summary its channel
   * brings; a conversation without one keeps the change in its draft. The
   * result is the user's last choice for new conversations.
   */
  async function changeModel(
    conversation: Conversation,
    change: ModelSettingsChange,
  ): Promise<void> {
    if (conversation.persistence !== 'synced') {
      conversation.model = applyModelChange(shownSettings(conversation), change)
      saveDrafts()
      rememberModel(conversation)
      return
    }
    // A record without a model takes the whole settings, as a first send
    // writes them.
    const recorded = product.snapshot?.conversations.some(
      (item) => item.id === conversation.id && item.model,
    )
    const settings = recorded ? null : applyModelChange(shownSettings(conversation), change)
    // The record shows the new settings at once; a refused change gives the stored ones back.
    const shown = product.change(`conversation:${conversation.id}`, (state) =>
      withConversation(state, conversation.id, (record) => ({
        ...record,
        model: settings ?? applyModelChange(record.model ?? shownSettings(conversation), change),
      })))
    const signal = lifetime.signal
    try {
      const changed = await writes.run(conversation.id, async () => {
        signal.throwIfAborted()
        return patch(conversation.id, settings ? settingsPatch(settings) : change, shown)
      })
      if (!changed) {
        return
      }
      signal.throwIfAborted()
      rememberModel(conversation)
      // A conversation opened without a usable model opens now with this
      // one, outside the queue: its opening waits for an archive or a
      // restore of it that may stand behind this write.
      const cached = cache.get(conversation.id)
      if (cached && !cached.runtime) {
        await reloadSession(conversation.id)
      }
    } catch (error) {
      shown.drop()
      report(t('failures.changeModel'), error)
    }
  }

  /**
   * The user makes `workflow` the conversation's Workflow menu choice
   * (`conversations.md` § A conversation's workflow), which its project's next
   * conversation starts with; settles once it is written or refused.
   */
  async function changeWorkflow(id: string, workflow: WorkflowChoice): Promise<void> {
    const conversation = items.value.find((item) => item.id === id)
    if (conversation) {
      remember(conversation, { lastWorkflow: workflow })
    }
    await applyWorkflow(id, workflow)
  }

  /**
   * A new conversation that started with a workflow its directory no longer
   * has goes on with No Subagents once the directory's workflows are read
   * (`web-api-settings.md` § User preferences); nothing is remembered.
   */
  function settleStartingWorkflow(id: string, list: WorkflowList): void {
    const conversation = items.value.find((item) => item.id === id)
    if (!conversation || conversation.persistence !== 'draft') {
      return
    }
    const workflow = startingWorkflow(conversation.workflow, list)
    if (!sameWorkflow(workflow, conversation.workflow)) {
      conversation.workflow = workflow
      saveDrafts()
    }
  }

  /**
   * Makes `workflow` the conversation's Workflow menu choice: it shows at
   * once, and a write that fails or is refused gives the stored choice
   * back; settles once it is written or refused. A conversation without a
   * record keeps it in its draft, which its first send gives the record,
   * and one whose record is on its way applies it as soon as the record is
   * made, before its first message goes.
   */
  async function applyWorkflow(id: string, workflow: WorkflowChoice): Promise<void> {
    const conversation = items.value.find((item) => item.id === id)
    if (conversation && conversation.persistence !== 'synced') {
      conversation.workflow = workflow
      saveDrafts()
      return
    }
    const change = product.change(`conversation:${id}`, (state) =>
      withConversation(state, id, (record) => ({ ...record, workflow })))
    const signal = lifetime.signal
    await writes
      .run(id, () => {
        signal.throwIfAborted()
        return patch(id, { workflow }, change)
      })
      .catch((error) => {
        change.drop()
        report(t('failures.changeWorkflow'), error)
      })
  }

  /** Whether two choices of the Workflow menu are the same. */
  function sameWorkflow(a: WorkflowChoice, b: WorkflowChoice): boolean {
    return a.type === b.type && (a.type !== 'workflow' || (b.type === 'workflow' && a.id === b.id))
  }

  /** The conversation's model settings become the last choice where it belongs. */
  function rememberModel(conversation: Conversation): void {
    const settings = conversation.model
    if (!settings.providerId || !settings.modelId) {
      return
    }
    remember(conversation, { lastModel: { ...settings } })
  }

  /**
   * Saves an explicit choice made in `conversation` where it belongs
   * (`web-api-settings.md` § User preferences): on its project's workspace, which its
   * project's next conversation starts with, or in the preferences for one
   * outside projects.
   */
  function remember(conversation: Conversation, choice: RememberedChoices): void {
    if (conversation.projectId === null) {
      preferences.update(choice)
      return
    }
    resources.rememberInProject(conversation.projectId, choice)
  }

  /**
   * Sends the draft the way the composer chose (`turns.md` § Steer or
   * queue): as the next message, as a steer of the running turn, or into the
   * queue, which is a message sent while a turn runs. The message shows as
   * what it will be from the send, and goes after the messages still on
   * their way, never ahead of them.
   */
  async function send(conversation: Conversation, way: SendWay = 'send'): Promise<void> {
    if (
      conversation.archived ||
      // A file whose upload failed is retried or removed in the composer first.
      messageFiles(conversation).some((file) => isComposerFile(file) && file.phase === 'failed') ||
      (!conversation.draft.trim() && !conversation.attachmentIds.length && !conversation.comments.length)
    ) {
      return
    }
    // In the order the conversation reads them, as the pill lists them.
    const comments = inReadingOrder(conversation.comments, latestBlocks(conversation.history).map((block) => block.id))
    // The waiting comments go with the message, whether it steers or queues.
    const fileIds = messageFiles(conversation).map((file) => file.id)
    const pending = addPendingSend(conversation, way, conversation.draft, fileIds, { comments })
    conversation.draft = ''
    conversation.attachmentIds = []
    conversation.comments = []
    await deliverInOrder(conversation, pending)
  }

  /**
   * Send Now on a new comment (`comments-and-ask.md` § Comments): a message
   * holding this comment alone goes at once, as the composer's Return would
   * send one, steering or waiting in the queue while the agent works; the
   * comments waiting in the draft and its text and files stay as they are.
   */
  async function sendComment(conversation: Conversation, comment: MessageComment): Promise<void> {
    // Refused, it waits with the others rather than being lost.
    if (conversation.archived) {
      setComments(conversation, [...conversation.comments, comment])
      return
    }
    const way = chooseSendWay(agentWorks(conversation.phase, conversation.pendingSends), preferences.sendWhileRunning, false)
    const pending = addPendingSend(conversation, way, '', [], { comments: [comment] })
    commentSends.add(pending.id)
    try {
      await deliverInOrder(conversation, pending)
    } finally {
      commentSends.delete(pending.id)
    }
  }

  /**
   * Save for Later (`conversations.md` § Saved for later): the composer's message,
   * its files included, stands at the top of the list and leaves the
   * composer at once, under the id the page gives it, once its files are
   * uploaded; the backend keeps it and empties the draft with it.
   */
  async function saveForLater(conversation: Conversation): Promise<void> {
    if (
      conversation.archived ||
      messageFiles(conversation).some((file) => isComposerFile(file) && file.phase === 'failed') ||
      isEmptyMessage(conversation.draft, conversation.attachmentIds.length)
    ) {
      return
    }
    try {
      await uploaded(conversation)
    } catch (error) {
      report(t('failures.saveMessage'), error)
      return
    }
    await recordSync.saveForLater(conversation, crypto.randomUUID(), composerWriting(conversation).writing)
  }

  /** Resolves once every file of the composer's message is uploaded; rejects when one cannot be. */
  async function uploaded(conversation: Conversation): Promise<void> {
    await Promise.all(messageFiles(conversation).flatMap((file) => isComposerFile(file) ? [uploads.uploaded(file)] : []))
  }

  /**
   * Edit on a message of the list: it goes into the composer and leaves the
   * list at once; a message the composer holds is saved for later in its
   * place, its files uploaded first, so it is kept whole. A message not
   * saved goes into the composer the same way, the composer's message saved
   * for later first.
   */
  async function editSaved(conversation: Conversation, id: string): Promise<void> {
    if (conversation.archived) {
      return
    }
    try {
      await uploaded(conversation)
    } catch (error) {
      report(t('failures.editMessage'), error)
      return
    }
    const notSaved = conversation.notSaved.find((row) => row.id === id)
    const message = conversation.record.shown.value?.saved.find((each) => each.id === id)
    if (notSaved) {
      if (!isEmptyMessage(conversation.draft, conversation.attachmentIds.length)) {
        void recordSync.saveForLater(conversation, crypto.randomUUID(), composerWriting(conversation).writing)
      }
      conversation.notSaved = conversation.notSaved.filter((row) => row !== notSaved)
      // The comments waiting meanwhile stay; a message of the list has none.
      showWriting(conversation, { ...notSaved, comments: conversation.comments })
    } else if (message) {
      void recordSync.edit(conversation, message)
    } else {
      return
    }
    if (conversation.id === product.activeConversationId) {
      composerFocusRequests.value += 1
    }
  }

  /**
   * Send on a saved message: it goes the way the composer chose, `way`, as a
   * turn of its own, and shows in the conversation at once as a message on
   * its way, as one sent from the composer does, which leaves the list. The
   * page keeps where it came from, so a send interrupted by a reload goes
   * again from the list with the same turn; one another page sent or
   * deleted first is gone, and nothing is sent.
   */
  function sendSaved(conversation: Conversation, id: string, way: SendWay): void {
    const message = conversation.record.shown.value?.saved.find((each) => each.id === id)
    if (conversation.archived || !message) {
      return
    }
    const files = message.files.map((file) => draftAttachment(conversation, file))
    conversation.files = [
      ...conversation.files,
      ...files.filter((file) => !conversation.files.some((held) => held.id === file.id)),
    ]
    const pending = addPendingSend(conversation, way, message.text, files.map((file) => file.id), { saved: id })
    recordSync.hideSent(conversation, id, pending.id)
    void deliverInOrder(conversation, pending)
  }

  /**
   * Delete on a message of the list: it goes at once, and the toast's Undo
   * saves it again, under a new id. A message not saved goes from this page
   * alone. A refused deletion brings the message back, and the toast goes.
   */
  async function deleteSaved(conversation: Conversation, id: string): Promise<void> {
    const notSaved = conversation.notSaved.find((row) => row.id === id)
    if (notSaved) {
      conversation.notSaved = conversation.notSaved.filter((row) => row !== notSaved)
      showDeleted(() => {
        conversation.notSaved = [notSaved, ...conversation.notSaved.filter((row) => row.id !== id)]
      })
      return
    }
    const message = conversation.record.shown.value?.saved.find((each) => each.id === id)
    if (conversation.archived || !message) {
      return
    }
    const ended = recordSync.remove(conversation, id)
    const toast = showDeleted(() => void recordSync.resave(conversation, crypto.randomUUID(), message))
    if (await ended === 'refused') {
      dismissToast(toast)
    }
  }

  /** The toast a deletion shows, whose Undo runs `undo`. */
  function showDeleted(undo: () => void): string {
    return showToast({ title: t('savedDeleted'), tone: 'neutral', action: { label: shared('common.undo'), run: undo } })
  }

  /** Retry on a message not saved: it is saved again under its own id, and stands as saving meanwhile. */
  function retrySaved(conversation: Conversation, id: string): void {
    const row = conversation.notSaved.find((each) => each.id === id)
    if (conversation.archived || !row) {
      return
    }
    void recordSync.resave(conversation, row.id, row)
  }

  /**
   * Holds a message as sent and not yet confirmed, after those still on
   * their way, with the comments it carries or the message saved for later
   * it is; answers it as the conversation holds it.
   */
  function addPendingSend(
    conversation: Conversation,
    way: SendWay,
    text: string,
    fileIds: string[],
    { comments = [], saved }: { comments?: MessageComment[]; saved?: string } = {},
  ): PendingSend {
    if (conversation.persistence === 'draft') {
      conversation.persistence = 'pending'
    }
    const id = crypto.randomUUID()
    // Plain copies: the page keeps its messages on their way in the web browser's storage.
    const carried = comments.map((comment) => ({ ...comment }))
    conversation.pendingSends = [...conversation.pendingSends, { id, way, text, fileIds, comments: carried, error: null, ...(saved ? { saved } : {}) }]
    return conversation.pendingSends.find((pending) => pending.id === id)!
  }

  /**
   * Retry on a message whose delivery failed: it goes again with its id,
   * after the messages on their way, as a message. A steer the session
   * already holds is not sent again, and one it does not hold is answered
   * next, or queued while a turn runs or a message on its way starts one.
   */
  async function retrySend(conversation: Conversation, id: string): Promise<void> {
    const pending = conversation.pendingSends.find((each) => each.id === id)
    if (conversation.archived || pending?.error == null) {
      return
    }
    pending.way = agentWorks(conversation.phase, conversation.pendingSends) ? 'queue' : 'send'
    pending.error = null
    delete pending.requestId
    await deliverInOrder(conversation, pending)
  }

  /** Delivers the message once the conversation's messages sent before it are delivered or failed. */
  function deliverInOrder(conversation: Conversation, pending: PendingSend): Promise<void> {
    return deliveries.run(conversation.id, () =>
      deliver(conversation, pending, pending.way === 'steer' ? 'steer' : 'message'))
  }

  /**
   * Delivers the message the conversation holds as sent: waits for every
   * upload of its files, trying again one that failed, makes the
   * conversation's record when it has none, then gives the message to the
   * session with its id, which takes it once, as a message or as a steer. A
   * message never goes without a file of it (`composer.md` § Writing a
   * message): one that cannot be uploaded leaves it undelivered.
   */
  async function deliver(
    conversation: Conversation,
    pending: PendingSend,
    way: 'message' | 'steer',
  ): Promise<void> {
    const signal = lifetime.signal
    pending.error = null
    delete pending.requestId
    // The files in the order of their marks in the text.
    const files = pending.fileIds.flatMap((id) =>
      conversation.files.filter((file) => file.id === id),
    )
    saveDrafts()
    // A backend out of reach fails nothing: the record's request waits in the
    // HTTP client, and the message waits in the runtime for the conversation
    // to open, and goes with its id once the backend answers again
    // (`web-application.md` § A page of another build). Only a refusal is a
    // failed delivery.
    const current = () => conversation.pendingSends.some((each) => each.id === pending.id)
    try {
      if (pending.saved) {
        await deliverSaved(conversation, pending, pending.saved, way)
        return
      }
      if (files.length !== pending.fileIds.length || files.length !== markCount(pending.text)) {
        throw new Error('A file of this message is no longer available. Attach it again.')
      }
      const [references] = await Promise.all([
        Promise.all(files.map(async (file): Promise<ClientContent> => {
          if (!isComposerFile(file)) {
            return { type: 'remote_file', deviceId: file.deviceId, path: file.path }
          }
          const upload = await uploads.uploaded(file)
          return { type: 'upload', ref: upload.id, fileName: file.name }
        })),
        persistConversation(conversation),
      ])
      // Each file where its capsule stands in the text.
      const content = joinMessageContent(pending.text, references.map((reference) => [reference]))
      const runtime = await runtimeFor(conversation)
      await (way === 'steer' ? runtime.steer(content, pending.id, pending.comments) : runtime.submit(content, pending.id, pending.comments))
      clearSubmission(conversation, pending.id)
    } catch (error) {
      // The page let the conversations go, as a sign-out does, or the message is no longer one to send.
      if (!signal.aborted && current()) {
        const wish = wishes.get(pending.id)
        // A message not delivered is no steer or queued message to act on.
        wishes.delete(pending.id)
        stopRemoving(conversation, pending.id)
        if (wish === 'recall' && pending.saved) {
          // One from the list the user took back meanwhile is listed again.
          dropSubmission(conversation, pending.id)
          recordSync.unsend(conversation, pending.id)
        } else if (wish === 'recall' && isEmptyMessage(conversation.draft, conversation.attachmentIds.length) && conversation.comments.length === 0) {
          // One the user took back meanwhile goes back into the empty
          // composer as it was, its files and its comments with it.
          conversation.pendingSends = conversation.pendingSends.filter((each) => each.id !== pending.id)
          refill(conversation, pending.text, pending.fileIds)
          setComments(conversation, pending.comments)
        } else if (commentSends.has(pending.id)) {
          // A comment's Send Now not delivered: its comment waits with the
          // others again, the draft and they as they were, and says why.
          conversation.pendingSends = conversation.pendingSends.filter((each) => each.id !== pending.id)
          setComments(conversation, [...conversation.comments, ...pending.comments])
          report(t('failures.sendComment'), error)
        } else {
          // Its row comes back, undelivered with the reason and the request
          // it was refused on: a draft the composer holds is not to be
          // replaced, and the message is not lost.
          pending.error = errorMessage(error)
          const requestId = requestIdOf(error)
          if (requestId !== null) {
            pending.requestId = requestId
          }
        }
      }
    }
    if (!signal.aborted) {
      saveDrafts()
    }
  }

  /**
   * Delivers a message from the list, `saved`, as the turn `pending.id`: the
   * backend sends it and takes it off the list. One gone from the list,
   * which another page sent or deleted, is sent by nobody, and its row here
   * goes too.
   */
  async function deliverSaved(conversation: Conversation, pending: PendingSend, saved: string, way: 'message' | 'steer'): Promise<void> {
    // A message saved a moment ago is on the backend's list only once its
    // save, made before the send, is answered.
    await recordSync.settled(conversation)
    const runtime = await runtimeFor(conversation)
    const delivery = way === 'steer' ? 'steer' : pending.way === 'queue' ? 'queue' : 'message'
    if (await runtime.sendSaved(saved, pending.id, delivery)) {
      clearSubmission(conversation, pending.id)
    } else {
      dropSubmission(conversation, pending.id)
    }
    recordSync.sent(conversation, pending.id)
    saveDrafts()
  }

  /** A sent message waits for a file's upload, which closing the page would stop. */
  const waitingForUploads = computed(() => items.value.some((conversation) =>
    conversation.pendingSends.some((pending) => pending.error === null && conversation.files.some((file) =>
      pending.fileIds.includes(file.id) && isComposerFile(file) && !file.upload))))

  /**
   * Takes a file a pasted message holds as a blob of the user's into the
   * conversation's composer at once, attaching beside its capsule
   * (`ConversationUploads.claimFile`).
   */
  function claimFile(conversation: Conversation, file: CopiedStoredFile, picture: File | null) {
    return uploads.claimFile(conversation, file, picture)
  }

  /** Takes a file on a paired device a pasted message names, as many times as messages name it. */
  function carryDeviceFile(conversation: Conversation, file: CopiedDeviceFile): ProductAttachment {
    conversation.files.push({ ...composerRemoteAttachment({ host: file.host, path: file.path }), deviceId: file.deviceId })
    return conversation.files.at(-1)!
  }

  /**
   * How a message pasted into the conversation's composer brings its files
   * back: a stored file at once, attaching beside its capsule without its
   * bytes, and a device file by its device and path (`composer.md` § Writing
   * a message).
   */
  function pasteHost(conversation: Conversation): PasteHost {
    return {
      claim: (file, picture) => {
        const { item, kept } = claimFile(conversation, file, picture)
        return { capsule: composerCapsule(item), kept }
      },
      remote: (file) => composerCapsule(carryDeviceFile(conversation, file)),
    }
  }

  /** How many files a message's text holds a place for. */
  function markCount(text: string): number {
    return text.split(ATTACHMENT_MARK).length - 1
  }

  /** The files the message has, in the order of its capsules; the rest wait for an undo. */
  function messageFiles(conversation: Conversation): Conversation['files'] {
    return conversation.attachmentIds.flatMap((id) =>
      conversation.files.filter((file) => file.id === id),
    )
  }

  /** The session holds the message `id`: it leaves the messages on their way, and a wish kept for it is carried out. */
  function clearSubmission(conversation: Conversation, id: string): void {
    if (!dropSubmission(conversation, id)) {
      return
    }
    const wish = wishes.get(id)
    wishes.delete(id)
    if (wish) {
      carryOut(conversation, id, wish)
    }
  }

  /**
   * Takes the message `id` out of those on their way, with its files;
   * answers whether it was one.
   */
  function dropSubmission(conversation: Conversation, id: string): boolean {
    const pending = conversation.pendingSends.find((each) => each.id === id)
    if (!pending) {
      return false
    }
    conversation.pendingSends = conversation.pendingSends.filter((each) => each.id !== id)
    for (const fileId of pending.fileIds) {
      removeFile(conversation, fileId)
    }
    // The message is gone: what its composer still held for an undo goes with it.
    releaseSpare(conversation)
    saveDrafts()
    // Once no message is on its way, the backend has accepted them all: the
    // draft they were written in goes too, everywhere, and a change another
    // page made meanwhile stays to restore.
    void recordSync.save(conversation).catch((error) => report(t('failures.clearDraft'), error))
    return true
  }

  /**
   * Keeps Recall or Send Now pressed on a message on its way as the
   * user's wish, when `id` is one; false for any other.
   */
  function keepWish(conversation: Conversation, id: string, wish: 'recall' | 'sendNow'): boolean {
    if (!conversation.pendingSends.some((pending) => pending.id === id && pending.error === null)) {
      return false
    }
    wishes.set(id, wish)
    return true
  }

  /**
   * Carries out the wish kept for the message `id`, which the session now
   * holds: on the steer or the queued message it became, as the controls on
   * those do. A steer the session refused went as a message, which may have
   * joined the queue; one already in the transcript is past both, which a
   * removal tells.
   */
  function carryOut(conversation: Conversation, id: string, wish: 'recall' | 'sendNow'): void {
    if (wish === 'recall') {
      carryOutRecall(conversation, id)
    } else if (conversation.pendingSteers.some((steer) => steer.id === id)) {
      action(conversation, (runtime) => runtime.interruptPendingSteer(id))
    } else if (conversation.queue.some((message) => message.id === id)) {
      action(conversation, (runtime) => runtime.sendQueuedNow(id))
    }
  }

  /**
   * Recall on a steer or a queued message, or on a message on its way
   * that will be one (`turns.md` § Steer or queue): its row goes at once,
   * and the rest follows once the session holds the message.
   */
  function recall(conversation: Conversation, id: string): void {
    if (conversation.removing.includes(id)) {
      return
    }
    conversation.removing = [...conversation.removing, id]
    if (!keepWish(conversation, id, 'recall')) {
      carryOutRecall(conversation, id)
    }
  }

  /**
   * Takes back the message `id` the session holds: the message goes back to
   * the user at once (`takeBack`), and its removal follows. A removal that
   * fails takes back what `takeBack` did. One the agent beat, as when it read
   * the steer meanwhile, is no failure: the message shows where the agent
   * took it, with no toast (`turns.md` § Steer or queue); any other failure
   * brings the row back with a toast.
   */
  function carryOutRecall(conversation: Conversation, id: string): void {
    const steer = conversation.pendingSteers.find((each) => each.id === id)
    const waiting = steer ?? conversation.queue.find((each) => each.id === id)
    const removal = runtimeFor(conversation).then((runtime) => runtime.removeWaiting(id))
    const back = waiting
      ? takeBack(conversation, { copy: contentCopy(waiting.content), comments: waiting.comments ?? [] }, steer ? 'steer' : 'queue', removal)
      : null
    void removal
      .catch((error) => {
        if (back) {
          giveBack(conversation, back)
        }
        if (!(error instanceof RemovalRefusedError)) {
          report(t('failures.recallMessage'), error)
        }
      })
      .finally(() => stopRemoving(conversation, id))
  }

  /**
   * Gives a message Recall took back to the user (`turns.md` § Steer or
   * queue): into the composer when it is empty, its files coming back as a
   * paste brings them, without their bytes, and its comments waiting again
   * (`comments-and-ask.md` § Sending them); otherwise, comments waiting
   * there included, the draft stays as it is, and a toast's Undo sends the
   * message again the way it was sent, its comments with it, once its
   * removal is done.
   */
  function takeBack(
    conversation: Conversation,
    message: { copy: CopiedMessage; comments: MessageComment[] },
    way: 'steer' | 'queue',
    removal: Promise<void>,
  ): TakenBack {
    if (!isEmptyMessage(conversation.draft, conversation.attachmentIds.length) || conversation.comments.length > 0) {
      // A removal that fails dismisses this one.
      const toast = showRecalled(() => void removal.then(() => resend(conversation, message, way), () => {}))
      return { kind: 'recalled', toast }
    }
    const comments = message.comments.map((comment) => ({ ...comment }))
    if (comments.length > 0) {
      setComments(conversation, comments)
    }
    return { kind: 'refilled', ...pasteAtEnd(conversation, message.copy), comments }
  }

  /**
   * Pastes the copied message at the end of the draft, as a paste there
   * adds it: after the draft's text, as a paragraph of its own, its files
   * coming back as a paste brings them, without their bytes; a file the
   * paste cannot use stands as its name, and a toast names it. The composer
   * shows the draft anew, in focus when it shows.
   */
  function pasteAtEnd(conversation: Conversation, copy: CopiedMessage): { text: string; fileIds: string[] } {
    const pasted = pasteMessage(copy, null, pasteHost(conversation))
    const fileIds = [...conversation.attachmentIds, ...pasted.capsules.map((capsule) => capsule.id)]
    const draft = conversation.draft.trimEnd()
    const joined = draft === '' ? pasted.markdown : `${draft}\n\n${pasted.markdown}`
    // As the editor writes the message out, so that a refusal finds it unchanged.
    const text = serializeUserMarkdown(parseUserMarkdown(joined, fileIds)).markdown
    refill(conversation, text, fileIds)
    for (const { id, kept } of pasted.attaching) {
      void kept.then((stays) => {
        const file = conversation.files.find((each) => each.id === id)
        if (!stays && file) {
          unattached(conversation, file)
        }
      })
    }
    const missing = missingFilesToast(pasted.missing)
    if (missing) {
      showToast({ ...missing, tone: 'danger' })
    }
    return { text, fileIds }
  }

  /**
   * Send to Conversation's end (`web-application.md` § Context menus): the
   * words go to the end of the conversation's draft as pasting them there
   * would add them, and nothing is sent. A conversation just opened takes
   * them once its draft is the one to change: restored from this web
   * browser and, for one the backend has, read from it, so they never land
   * on a draft that the read then replaces; one that fails to open, or a
   * sign-out, drops them.
   */
  function appendToDraft(conversation: Conversation, words: CopiedMessage): void {
    const known = () =>
      restored.has(conversation.id) && (conversation.persistence !== 'synced' || conversation.record.revision('draft') >= 0)
    if (known()) {
      pasteAtEnd(conversation, words)
      return
    }
    const signal = lifetime.signal
    const stop = watch(
      () => (known() ? 'known' : conversation.load === 'failed' ? 'failed' : 'waiting'),
      (state) => {
        if (state === 'waiting') {
          return
        }
        release()
        if (state === 'known') {
          pasteAtEnd(conversation, words)
        }
      },
    )
    const release = () => {
      stop()
      signal.removeEventListener('abort', release)
    }
    signal.addEventListener('abort', release)
  }

  /** The empty composer holds the message `text` with the files `fileIds`, the caret at its end, in focus when it shows. */
  function refill(conversation: Conversation, text: string, fileIds: string[]): void {
    conversation.draft = text
    arrangeFiles(conversation, fileIds)
    conversation.draftShown += 1
    if (conversation.id === product.activeConversationId) {
      composerFocusRequests.value += 1
    }
  }

  /**
   * Takes back what `takeBack` did once the removal failed, as the row comes
   * back: the toast goes, and the composer it refilled is empty again,
   * unless the user changed it since, when it stays as they left it; the
   * comments it gave back leave the draft, those the user changed since
   * included.
   */
  function giveBack(conversation: Conversation, back: TakenBack): void {
    if (back.kind === 'recalled') {
      dismissToast(back.toast)
      return
    }
    if (back.comments.length > 0) {
      setComments(conversation, conversation.comments.filter((each) => !back.comments.some((taken) => taken.id === each.id)))
    }
    if (conversation.draft !== back.text || conversation.attachmentIds.join('\n') !== back.fileIds.join('\n')) {
      return
    }
    conversation.draft = ''
    arrangeFiles(conversation, [])
    releaseSpare(conversation)
    conversation.draftShown += 1
  }

  /**
   * Undo on the toast of a message Recall took back: it is sent again the
   * way it was, its files attached again without their bytes and its
   * comments with it, as one the composer sent.
   */
  function resend(conversation: Conversation, message: { copy: CopiedMessage; comments: MessageComment[] }, way: 'steer' | 'queue'): void {
    const pasted = pasteMessage(message.copy, null, pasteHost(conversation))
    const pending = addPendingSend(conversation, way, pasted.markdown, pasted.capsules.map((capsule) => capsule.id), { comments: message.comments })
    void deliverInOrder(conversation, pending)
  }

  function stopRemoving(conversation: Conversation, id: string): void {
    conversation.removing = conversation.removing.filter((each) => each !== id)
  }

  function reconcileSubmission(conversation: Conversation): void {
    for (const pending of conversation.pendingSends) {
      if (hasAcceptedSubmission(conversation, pending.id)) {
        clearSubmission(conversation, pending.id)
      }
    }
  }

  /** Runs `operation` on the conversation's runtime, reporting a failure; answers whether it succeeded. */
  async function attempt(
    conversation: Conversation,
    operation: (runtime: ConversationRuntime) => Promise<void>,
  ): Promise<boolean> {
    try {
      await operation(await runtimeFor(conversation))
      return true
    } catch (error) {
      if (error instanceof ConversationSocketError) {
        // The socket dropped before the session answered: the page cannot
        // tell whether it got the request, and never sends it again
        // (`web-application.md` § A request the drop interrupts). The drop
        // itself is reported where the socket was replaced.
        report(t('failures.connectionLost'), error, {
          expected: true,
          message: t('failures.connectionLostMessage'),
        })
      } else if (!isRecordedTurnFailure(error)) {
        // A turn that ran and failed is already a record in the transcript.
        report(t('failures.requestNotAccepted'), error)
      }
      return false
    }
  }

  function action(
    conversation: Conversation,
    operation: (runtime: ConversationRuntime) => Promise<void>,
  ): void {
    void attempt(conversation, operation)
  }

  /**
   * Stops the conversation's turn. A Stop pressed while the message that
   * starts the turn is still on its way stops that turn once the session
   * holds the message, after the messages sent before the press; a message
   * that was not delivered started nothing.
   */
  async function stop(conversation: Conversation): Promise<boolean> {
    const starting = turnStartingMessage(conversation.phase, conversation.pendingSends)?.id
    await deliveries.run(conversation.id, async () => {})
    if (starting !== undefined && conversation.pendingSends.some((pending) => pending.id === starting)) {
      return true
    }
    return attempt(conversation, (runtime) => runtime.abort())
  }

  /**
   * Resolves true once the conversation runs nothing: its root is idle and
   * its subagents and commands have ended, as their phases announce; false
   * when the store stops first.
   */
  function untilIdle(conversation: Conversation): Promise<boolean> {
    const active = () => workToStop(conversation) !== null
    if (!active()) {
      return Promise.resolve(true)
    }
    const signal = lifetime.signal
    return new Promise((resolve) => {
      const unwatch = watch(active, (now) => {
        if (!now) {
          settle(true)
        }
      }, { flush: 'sync' })
      const onAbort = () => settle(false)
      signal.addEventListener('abort', onAbort, { once: true })
      function settle(idle: boolean): void {
        unwatch()
        signal.removeEventListener('abort', onAbort)
        resolve(idle)
      }
    })
  }

  /**
   * Stops what the conversation's agent does, its running subagents and
   * commands included, and answers once the session says it is idle; a
   * conversation that is idle already answers at once. False when a stop was
   * refused or the store stopped before the session was idle.
   */
  async function stopWork(conversation: Conversation): Promise<boolean> {
    if (runningSubagents(conversation.subagents).length > 0
      && !await attempt(conversation, (runtime) => runtime.abortSubagents())) {
      return false
    }
    for (const terminal of runningTerminals(conversation.terminals)) {
      if (!await attempt(conversation, (runtime) => runtime.abortTerminal(terminal.id))) {
        return false
      }
    }
    if (conversation.phase !== 'idle' && !await stop(conversation)) {
      return false
    }
    return untilIdle(conversation)
  }

  /**
   * Moves the conversation to run on `target` (`hosts.md` § Where a
   * conversation runs); with `notifyAgent`, the backend tells its agent.
   */
  function switchTarget(id: string, target: ConversationSummary['target'], notifyAgent = false): Promise<boolean> {
    const conversation = items.value.find((item) => item.id === id)
    // A move into a project on the Host it runs on shows at once, as a
    // change of the conversation's record; the write follows (`web-application.md`
    // § Responding to the user). Any other move waits for its Host.
    if (conversation?.persistence === 'synced' && target.kind === 'workspace'
      && executionFor({ target, cwd: '' }).deviceId === executionFor(conversation).deviceId) {
      void applyBatch([id], notifyAgent ? { target, notifyAgent } : { target }, true, 'move')
      return Promise.resolve(true)
    }
    return changeConversations(
      [id],
      async () => (await applyBatch([id], notifyAgent ? { target, notifyAgent } : { target }, false, 'move')).success,
      false,
    )
  }

  /**
   * Moves a conversation whose agent works: stops the work, and sends the
   * move only once the session says it is idle, since the backend refuses a
   * move while the tree is busy. A turn that ended meanwhile moves at once.
   */
  async function switchTargetAfterStop(
    conversation: Conversation,
    target: ConversationSummary['target'],
    notifyAgent: boolean,
  ): Promise<boolean> {
    return await stopWork(conversation) && switchTarget(conversation.id, target, notifyAgent)
  }

  function stopAll(): void {
    copies.clear()
    pendingChanges.value = []
    cache.clear()
    earlyReads.clear()
    restoringLocalDrafts.value = false
    madeHere.clear()
    deliveries = new SerialQueue()
    wishes.clear()
    recordSync.stop()
    lifetime.abort()
    lifetime = new AbortController()
    uploads.dispose(items.value)
    items.value = []
    deleted.clear()
    restored.clear()
    savedDrafts.clear()
    storageReported.clear()
  }

  return {
    items,
    listed,
    deleted,
    listStatus,
    composerFocusRequests,
    appendToDraft,
    reveal,
    history,
    activate,
    create,
    fork,
    reorder,
    rename,
    markRead,
    reloadList: () => product.reconnect(),
    reloadSession,
    pin: async (ids: string[], pinned: boolean) =>
      (await applyBatch(ids, { pinned }, true, pinned ? 'pin' : 'unpin')).success,
    archive,
    restore,
    remove,
    switchTarget,
    switchTargetAfterStop,
    pendingChanges,
    detachHost: (conversation: Conversation, deviceId: string) =>
      changeConversations(
        [conversation.id],
        () => detachHost(conversation, deviceId),
        undefined,
      ),
    reloadPlugins: (conversation: Conversation) =>
      changeConversations(
        [conversation.id],
        () => reloadPlugins(conversation),
        undefined,
      ),
    changeModel,
    changeWorkflow,
    settleStartingWorkflow,
    send,
    retrySend,
    commenting,
    clearComments,
    editVersion: (conversation: Conversation) =>
      cache.get(conversation.id)?.runtime?.transcriptVersion() ?? null,
    submitEdit: (conversation: Conversation) => submitMessageEdit(editHost(conversation)),
    regenerate: (conversation: Conversation, request: MessageEditRequest) =>
      regenerateMessage(editHost(conversation), request),
    switchVersion: (conversation: Conversation, blockId: string, to: number) =>
      action(conversation, (runtime) => runtime.switchVersion(blockId, to)),
    addFiles,
    claimFile,
    waitingForUploads,
    arrangeFiles,
    retryFile,
    saveDrafts,
    keepLastWords,
    /** Writes the copies for opening offline that wait for their delay, as the page hides; resolves once written. */
    keepCopies: () => copies.keep(),
    /** Saves the drafts that wait for typing to pause: when the page is hidden, or, with `keepalive`, closes. */
    flushDrafts: (keepalive = false) => recordSync.flush(items.value, keepalive),
    saveForLater,
    editSaved,
    sendSaved,
    deleteSaved,
    retrySaved,
    restoreReplaced: (conversation: Conversation) =>
      void recordSync.act(conversation, 'restore').catch((error) => report(t('failures.restoreDraft'), error)),
    dismissReplaced: (conversation: Conversation) =>
      void recordSync.act(conversation, 'dismiss').catch((error) => report(t('failures.dismissDraft'), error)),
    initialize,
    stopAll,
    abortSubagents: (conversation: Conversation) =>
      action(conversation, (runtime) => runtime.abortSubagents()),
    abortSubagent: (conversation: Conversation, id: string) =>
      action(conversation, (runtime) => runtime.abortSubagent(id)),
    showSubagents,
    /**
     * Whether the page has, or is opening, a session of the conversation,
     * which sends a live child's transcript once the page shows it
     * (`subagents.md` § Protocol): none when the opening started none, as
     * for a conversation whose model is gone, or failed.
     */
    streamsSubagents: (conversation: Conversation) =>
      conversation.load === 'opening' || (cache.get(conversation.id)?.runtime != null && conversation.load !== 'failed'),
    abortTerminal: (conversation: Conversation, id: string) =>
      action(conversation, (runtime) => runtime.abortTerminal(id)),
    start: (conversation: Conversation) =>
      action(conversation, (runtime) => runtime.resume()),
    stop,
    /** Compacts the root's context, or the live subagent `node`'s, as its context card asks. */
    compact: (conversation: Conversation, node?: string) =>
      action(conversation, (runtime) => runtime.compact(node)),
    // Send Now on a steer or a queued message still on its way waits for the
    // session to hold it; on any other, it goes at once. Recall takes the
    // row away at once in either case.
    recall,
    pasteHost,
    sendQueued: (conversation: Conversation, id: string) =>
      {
        if (!keepWish(conversation, id, 'sendNow')) {
          action(conversation, (runtime) => runtime.sendQueuedNow(id))
        }
      },
    interruptWithSteer: (conversation: Conversation, id: string) =>
      {
        if (!keepWish(conversation, id, 'sendNow')) {
          action(conversation, (runtime) => runtime.interruptPendingSteer(id))
        }
      },
  }
})
