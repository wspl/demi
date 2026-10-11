import { computed, reactive, ref, toRaw, watch } from 'vue'
import { defineStore } from 'pinia'
import { SerialQueue, deferred, moveBefore } from '@demicodes/utils'
import type { HeadlineText, TitleText } from '@demicodes/web-ui/ui/ui-text'
import type { ClientContent } from '@demicodes/protocol'
import { ConversationCache, type CachedConversation } from '@demicodes/web-ui/agent/conversation-cache'
import type { TranscriptReveal } from '@demicodes/web-ui/agent/history'
import { ConversationRuntime, isRecordedTurnFailure } from '@demicodes/web-ui/agent/conversation-runtime'
import {
  regenerateMessage,
  restoreMessageEdit,
  sentEditRequest,
  submitMessageEdit,
  type MessageEditHost,
  type MessageEditRequest,
} from '@demicodes/web-ui/agent/message-editing'
import { reportError } from '@demicodes/web-ui/infra/errors'
import { dismissToast, showToast } from '@demicodes/web-ui/infra/toast'
import { showArchived, updateArchived } from '@demicodes/web-ui/sidebar/archived-toast'
import { showRefused, type Refusal } from '@demicodes/web-ui/sidebar/refused-toast'
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
import type { SendWay } from '@demicodes/web-ui/agent/send-way'
import {
  composerAttachmentFromFile,
  composerRemoteAttachment,
  isComposerFile,
  remoteAttachmentError,
} from '@demicodes/web-ui/agent/message-input/attachments'
import { missingFilesToast, type CopiedDeviceFile, type CopiedStoredFile } from '@demicodes/web-ui/agent/message-input/copied-message'
import { ATTACHMENT_MARK, parseUserMarkdown, serializeUserMarkdown } from '@demicodes/web-ui/markdown/user-markdown'
import { connectConversationClient, takeConnection } from '@demicodes/web-ui/transport/conversation-socket'
import { loadDraft } from '../api/drafts'
import { ApiError, apiRequest, jsonBody, readResponse } from '../api/client'
import {
  attachedHostsSchema,
  batchAnswerSchema,
  createdConversationSchema,
  conversationUpdateSchema,
  subagentsSchema,
  transcriptPageSchema,
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
} from '../api/generated/web-api'
import { joinMessageContent } from '@demicodes/web-ui/agent/message-input/message-content'
import { blobPicture, createConversationUploads, sendingFileIds } from './uploads'
import { useResources } from '../state/resources'
import { useProduct, type ProductChange } from '../state/product'
import { usePreferences } from '../state/preferences'
import { useSession } from '../auth/session'
import type { Conversation, PendingSend, ProductAttachment } from '../state/types'
import { applyConversationEvent, updateLiveStatus } from './activity'
import { createDraftSync, hasUnsavedDraft } from './draft-sync'
import { createHistoryReads } from './history'
import { EMPTY_TRANSCRIPT, addPage } from '@demicodes/web-ui/transport/protocol'
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
  return conversation.persistence === 'draft' && !conversation.draft.trim() && !conversation.attachmentIds.length
}

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
   * (`product.md` § Conversations and projects).
   */
  const listed = computed(() => items.value.filter((item) =>
    !item.archived && (!isEmptyDraft(item) || item.id === product.activeConversationId)))
  /**
   * The writes on their way, by the conversation they change, or by
   * `conversation_order` for a move: each conversation's go one at a time in
   * order, and a write of several waits for each of them
   * (`web-application.md` § Responding to the user).
   */
  const writes = new SerialQueue()
  const restored = new Set<string>()
  /** Each conversation's draft as this page last saved or restored it, in the shape `changedDraft` compares. */
  const savedDrafts = new Map<string, string>()
  const uploads = createConversationUploads(saveDrafts, (error) =>
    report('Could Not Upload the Attachment', error),
  )
  const { uploadFile, addFiles, removeFile, releaseSpare, arrangeFiles, retryFile } = uploads
  const draftSync = createDraftSync({
    apply: showDraft,
    report,
  })
  let lifetime = new AbortController()
  const cache = new ConversationCache()
  /** The history a conversation's page reads beyond its opening (`web-application.md` § Transcript windows). */
  const history = createHistoryReads(
    () => lifetime.signal,
    (conversation) => cache.get(conversation.id)?.runtime != null,
  )
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
   * message never overtakes the one before it (`product.md` § Steer or
   * queue).
   */
  let deliveries = new SerialQueue()
  /**
   * Remove or Send Now pressed on a steer or a queued message still on its
   * way, by its message id: the user's latest wish, carried out once the
   * session holds the message (`web-application.md` § Responding to the
   * user).
   */
  const wishes = new Map<string, 'remove' | 'sendNow'>()
  /** The draft storage failures this page reported, each once. */
  const storageReported = new Set<'save' | 'restore' | 'unreadable'>()

  // A failed operation is a toast; server state is never replaced by a message.
  function report(title: HeadlineText, error: unknown): void {
    if (error instanceof DOMException && error.name === 'AbortError') {
      return
    }
    if (!lifetime.signal.aborted) {
      reportError(title, error, { userVisible: true })
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
        reportError('Could Not Save Drafts', error, {
          userVisible: true,
          message: 'Your drafts stay on this page, but this web browser could not store them.',
        })
        break
      case 'restore':
        reportError('Could Not Restore Drafts', error, {
          userVisible: true,
          message: 'This web browser could not read the drafts it stored.',
        })
        break
      case 'unreadable':
        reportError('Could Not Restore a Draft', error, {
          userVisible: true,
          detail,
          message: 'A draft stored in this web browser could not be read and was set aside. Your other drafts are restored.',
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
    // user's own Stop is an abort (`product.md` § Recovering an unfinished turn).
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
      | 'target'
      | 'revision'
      | 'readRevision'
      | 'unread'
      | 'cwd'
      | 'titleCurrent'
      | 'titleGenerating'
      | 'pluginsChanged'
      | 'permissionRequests'
      | 'createdAt'
      | 'updatedAt'
    >,
  ) {
    return {
      id: record.id,
      title: record.title,
      pinned: record.pinned,
      archived: record.archived,
      target: record.target,
      projectId:
        record.target.kind === 'workspace' ? record.target.workspaceId : null,
      cwd: record.cwd,
      revision: record.revision,
      readRevision: record.readRevision,
      unread: record.unread,
      titleCurrent: record.titleCurrent,
      titleGenerating: record.titleGenerating,
      pluginsChanged: record.pluginsChanged,
      needsYou: record.permissionRequests > 0,
      createdAt: record.createdAt,
      updatedAt: record.updatedAt,
    }
  }

  /**
   * A conversation shows the model settings its record holds, whichever page
   * or device changed them last (`web-api.md` § Sidebar mutations and read
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
      resources.modelsFor(),
      conversation.model.providerId,
      conversation.model.modelId,
    )
    if (!pick.providerId || !pick.modelId) {
      return conversation.model
    }
    return { ...conversation.model, providerId: pick.providerId, modelId: pick.modelId }
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
      lastError: null,
      draft: '',
      savedDraft: null,
      draftBase: null,
      draftShown: 0,
      files: [],
      attachmentIds: [],
      pendingSends: [],
      removing: [],
      messageEdit: null,
      scroll: null,
      attachedHosts: [],
      // Its opening reads the hosts at least this new.
      hostsRevision: record.hostsRevision,
      subagents: [],
      terminals: [],
      load: 'loading',
      pendingAction: null,
      failures: {},
      contextUsage: null,
    }
  }

  /**
   * Keeps everything the conversation shows as its archive flag changes
   * (`product.md` § Conversations and projects): archived, it lets its
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
          draftSync.changed(conversation)
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
        // Another page saved the draft since this one read it.
        if (current.savedDraft && record.draftRevision > current.savedDraft.revision) {
          void draftSync.read(current).catch((error) => report('Could Not Read the Draft', error))
        }
        // Another page or a host's shell changed the hosts since this one
        // read them; a conversation not opened reads them when it opens.
        if (record.hostsRevision > current.hostsRevision) {
          current.hostsRevision = record.hostsRevision
          if (cache.get(current.id)) {
            void readHosts(current).catch((error) => report('Could Not Read the Hosts', error))
          }
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
    const unsaved = local || hasUnsavedDraft(conversation)
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
      base: !local && unsaved ? conversation.draftBase ?? 0 : null,
      text: unsaved ? conversation.draft : '',
      // A conversation with a record holds its model settings there.
      model: local ? { ...conversation.model } : null,
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
    const userId = session.user?.id ?? product.snapshot?.user.id
    if (!userId) {
      return
    }
    const changed = items.value
      .filter((item) => restored.has(item.id))
      .map((item) => ({
        conversation: item,
        id: item.id,
        draft: persisted(item),
      }))
      .filter((entry) => changedDraft(entry.id, entry.draft))
    for (const entry of changed) {
      if (entry.conversation.persistence === 'synced') {
        draftSync.changed(entry.conversation)
      }
    }
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
      if (restored.has(item.id) && (item.persistence !== 'synced' || hasUnsavedDraft(item))) {
        writeLastWords(userId, item.id, {
          base: item.persistence === 'synced' ? item.draftBase ?? 0 : null,
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
          // A conversation the backend has not seen has no message to title.
          titleCurrent: true,
          titleGenerating: false,
          pluginsChanged: false,
          draftRevision: 0,
          panelRevision: 0,
          hostsRevision: 0,
          pluginRevisions: [],
          permissionRequests: 0,
          permissionsRevision: 0,
          status: 'idle',
          lastTurn: null,
          contextVersion: 0,
          revision: 0,
          readRevision: 0,
          unread: false,
        })
        conversation.persistence = draft.local.phase
        conversation.load = 'ready'
        // Drafts lead the list, newest first, whether the channel's state
        // is here yet or not (`product.md` § Conversations and projects).
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
          error: pending.error || 'Delivery was interrupted. Retry to confirm it.',
        }))
        conversation.messageEdit = restoreMessageEdit(draft.messageEdit ?? null)
        // A draft holds model settings only for a conversation without a
        // record.
        if (draft.model && conversation.persistence !== 'synced') {
          conversation.model = draft.model
        }
        conversation.scroll = draft.scroll
        // The composer's text is here only when the backend does not have
        // it; a synced conversation's composer shows the backend's draft
        // otherwise, once read.
        conversation.draft = draft.text
        conversation.draftBase = draft.base
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
        conversation.draft = words.text
        conversation.draftBase = words.base
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
        // again, and falls back as a paste does (`product.md` § Writing a message).
        const claimed = !!file.claim
        void uploadFile(file).catch((error) => {
          if (claimed && !file.file) {
            unattached(conversation, file)
          } else {
            report('Could Not Upload the Attachment', error)
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
      conversation.attachedHosts.find((attached) => attached.deviceId === file.deviceId)?.name ??
      resources.deviceById(file.deviceId)?.name ??
      file.deviceId
    return carried ?? { ...composerRemoteAttachment({ host, path: file.path }), deviceId: file.deviceId }
  }

  /** Shows a draft from outside, another page's or a restored one, in the conversation's composer. */
  function showDraft(conversation: Conversation, draft: ConversationDraft): void {
    const files = draft.files.map((file) => draftAttachment(conversation, file))
    conversation.draft = draft.text
    uploads.showFiles(conversation, files)
    conversation.draftShown += 1
  }

  /** Where a conversation's edit is kept while it is sent, and the session it is sent to. */
  function editHost(conversation: Conversation): MessageEditHost {
    return {
      get: () => conversation.messageEdit,
      set: (state) => { conversation.messageEdit = state },
      send: async (request) => {
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
    // A conversation the page shows already stays shown while it opens: the
    // first send opens the record it just created, and the composer the user
    // sent from must stay where it is, with the focus in it.
    if (!cache.get(conversation.id) && conversation.load !== 'ready') {
      conversation.load = conversation.persistence === 'synced' ? 'loading' : 'ready'
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
    const reads = {
      transcript: apiRequest(`${path}/transcript`, { signal }).then((response) => readResponse(response, transcriptPageSchema)),
      subagents: apiRequest(`${path}/subagents`, { signal }).then((response) => readResponse(response, subagentsSchema)),
      hosts: apiRequest(`${path}/hosts`, { signal }).then((response) => readResponse(response, attachedHostsSchema)),
      draft: loadDraft(id, signal),
    }
    // The opening that takes them reports their failures; one never taken has nobody to tell.
    for (const read of Object.values(reads)) {
      read.catch(() => {})
    }
    return reads
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
      const modelsLoaded = product.loadModels().catch(() => {})
      if (madeHere.delete(conversation.id)) {
        // Its record is what the page created: no transcript yet, the hosts
        // its answer listed, and the composer's draft.
        await draftRestored
        conversation.load = 'ready'
      } else {
        await readOpening(conversation, signal, draftRestored)
      }
      await modelsLoaded
      signal.throwIfAborted()
      const made = early
      early = null
      await startSession(conversation, entry, made)
    } catch (error) {
      discardEarly()
      if (!signal.aborted) {
        failOpening(conversation, error)
      }
      throw error
    }
  }

  /** The conversation did not open: the pane or the transcript's tail says why, with Retry. */
  function failOpening(conversation: Conversation, error: unknown): void {
    conversation.load = 'failed'
    conversation.lastError = error instanceof Error ? error.message : String(error)
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
      resources.modelsFor(),
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
      onEvent: (next) => {
        applyConversationEvent(conversation, next)
        reconcileSubmission(conversation)
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
      throw error
    }
  }

  /**
   * Reads what the page shows of a conversation it opens, all at once: its
   * transcript, which shows without waiting for its hosts, its hosts and
   * its draft.
   */
  async function readOpening(
    conversation: Conversation,
    signal: AbortSignal,
    draftRestored: Promise<void>,
  ): Promise<void> {
    const reads = earlyReads.get(conversation.id) ?? openingReads(conversation.id, signal)
    earlyReads.delete(conversation.id)
    const hostsRevision = conversation.hostsRevision
    // Taken after this web browser's own copy; a submission it confirms
    // clears the draft after it, so the transcript waits for it.
    const draftRead = draftRestored.then(() => draftSync.read(conversation, reads.draft))
    const hostsRead = reads.hosts.then((answer) => {
      signal.throwIfAborted()
      // A summary that raised the revision meanwhile started a newer read.
      if (conversation.hostsRevision === hostsRevision) {
        conversation.attachedHosts = answer.hosts
      }
    })
    hostsRead.catch(() => {})
    const [page, listed] = await Promise.all([reads.transcript, reads.subagents, draftRead])
    signal.throwIfAborted()
    // The latest page is the transcript's end until the stream says more.
    conversation.history = addPage(EMPTY_TRANSCRIPT, page, false)
    conversation.shownAt = null
    conversation.failures = page.failures ?? {}
    conversation.instructions = page.instructions
    conversation.summaries = Object.fromEntries(page.summaries.map((summary) => [summary.marker, summary.summaryTokens]))
    reconcileSubmission(conversation)
    conversation.terminals = []
    conversation.subagents = listed.subagents.map((subagent) => ({
      pendingCalls: [],
      waitingSince: null,
      id: subagent.subagentId,
      number: subagent.number,
      name: subagent.description,
      phase: subagent.phase,
      startedAt: subagent.startedAt,
      endedAt: subagent.endedAt ?? undefined,
      history: EMPTY_TRANSCRIPT,
      failures: {},
    }))
    updateLiveStatus(conversation)
    conversation.load = 'ready'
    await hostsRead
  }

  /**
   * Reads the hosts of an open conversation again, after a summary raised
   * their revision (`web-api.md` § Sidebar mutations and read state); an
   * answer that a later read overtook is dropped.
   */
  async function readHosts(conversation: Conversation): Promise<void> {
    const revision = conversation.hostsRevision
    const response = await apiRequest(`/conversations/${encodeURIComponent(conversation.id)}/hosts`, {
      signal: lifetime.signal,
    })
    const answer = await readResponse(response, attachedHostsSchema)
    if (conversation.hostsRevision === revision) {
      conversation.attachedHosts = answer.hosts
    }
  }

  async function runtimeFor(
    conversation: Conversation,
  ): Promise<ConversationRuntime> {
    if (conversation.archived) {
      throw new Error('Restore this conversation before sending.')
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
      throw new Error(
        conversation.lastError ?? 'No available model for this conversation.',
      )
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
        'Some fields were not updated.',
        failed.map((field) => `${field.field}: ${field.message}`).join('\n'),
        { userVisible: true, expected: true },
      )
    }
    return failed.length === 0
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
    action: TitleText,
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
          withConversation(state, id, (record) => ({ ...record, ...fields })))])
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
          const refusals: Refusal[] = []
          for (const item of result.results) {
            const change = shownChanges.get(item.id)
            // A refusal names the conversation as the list shows it now: its title may have come meanwhile.
            const title = items.value.find((conversation) => conversation.id === item.id)?.title ?? item.id
            if (item.status === 'refused') {
              change?.drop()
              refusals.push({ title, reasons: [item.message] })
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
              refusals.push({ title, reasons: failed.map((field) => field.message) })
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
      report('Could Not Update Conversations', error)
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
   * a toast whose Undo restores them (`product.md` § Conversations and
   * projects). Once the backend answered, the toast counts, and its Undo
   * restores, only those it archived; the refusal's own toast says why of
   * the others. Answers whether every one was archived.
   */
  async function archive(ids: string[]): Promise<boolean> {
    const shown = ids.filter((id) => items.value.some((item) => item.id === id && !item.archived))
    const done = applyBatch(ids, { archived: true }, true, 'Archive')
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
   * (`product.md` § Conversations and projects): they leave the list at
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
    const titles = new Map(synced.map((id) => [id, items.value.find((item) => item.id === id)?.title ?? id]))
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
            removal.drop()
            refusals.push({ title: titles.get(id) ?? id, reasons: [error instanceof Error ? error.message : String(error)] })
            continue
          }
          // The deletion lands; the channel brings it to every other page.
          removal.land()
          forget(id)
        }
        if (refusals.length) {
          showRefused('Delete', refusals)
        }
        return refusals.length === 0
      })
    } catch (error) {
      for (const removal of removals.values()) {
        removal.drop()
      }
      report('Could Not Delete Conversations', error)
      return false
    }
  }

  /** Brings archived conversations back at once; answers whether every one came back. */
  async function restore(ids: string[]): Promise<boolean> {
    const { success } = await applyBatch(ids, { archived: false }, true, 'Restore')
    return success
  }

  function create(projectId: string | null = null): string {
    const empty = items.value.find(
      (item) => isEmptyDraft(item) && !item.archived && item.projectId === projectId,
    )
    if (empty) {
      return empty.id
    }
    const now = new Date().toISOString()
    const conversation = newConversation({
      id: crypto.randomUUID(),
      title: 'New conversation',
      pinned: false,
      archived: false,
      target: projectId
        ? { kind: 'workspace', workspaceId: projectId }
        : { kind: 'cloud' },
      contextVersion: 0,
      revision: 0,
      readRevision: 0,
      unread: false,
      titleCurrent: true,
      titleGenerating: false,
      pluginsChanged: false,
      draftRevision: 0,
      panelRevision: 0,
      hostsRevision: 0,
      pluginRevisions: [],
      permissionRequests: 0,
      permissionsRevision: 0,
      createdAt: now,
      updatedAt: now,
      model: null,
      status: 'idle',
      lastTurn: null,
    })
    conversation.persistence = 'draft'
    conversation.model = initialModelSettings(preferences.lastModel)
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
   * target and its settings (`web-api.md` § Conversation creation and Fork):
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
    const listed = lookupSelectedModel(resources.modelsFor(), shown.providerId, shown.modelId)
    const settings = listed ? offeredSettings(shown, listed.model) : null
    const request: CreateConversation = {
      id: conversation.id,
      title: conversation.title,
      pinned: conversation.pinned,
      target: conversation.target,
      ...(settings ? settingsPatch(settings) : {}),
    }
    const response = await apiRequest('/conversations', {
      method: 'POST',
      signal,
      ...jsonBody(request),
    })
    const result = await readResponse(response, createdConversationSchema)
    signal.throwIfAborted()
    product.answered(sentAt, { type: 'conversation', conversation: result.conversation })
    conversation.persistence = 'synced'
    Object.assign(conversation, metadata(result.conversation))
    followRecordModel(conversation, result.conversation)
    conversation.hostsRevision = result.conversation.hostsRevision
    cache.delete(conversation.id)
    // A retry that found the record an earlier attempt made reads it.
    if (response.status === 201) {
      madeHere.add(conversation.id)
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
        report('Could Not Rename the Conversation', error)
      })
  }

  /** Asks for a title from the conversation as it stands (`product.md` § Conversation titles). */
  async function retitle(conversation: Conversation): Promise<void> {
    if (conversation.titleGenerating || conversation.persistence !== 'synced') {
      return
    }
    const id = conversation.id
    // The conversation says at once that it finds its title; from the
    // answer on, its summary says whether the request still runs.
    const change = product.change(`conversation:${id}`, (state) =>
      withConversation(state, id, (record) => ({ ...record, titleGenerating: true })))
    change.send()
    try {
      // The backend asks the model the conversation's record holds.
      await apiRequest(
        `/conversations/${encodeURIComponent(id)}/title`,
        { method: 'POST', signal: lifetime.signal },
      )
      change.land()
    } catch (error) {
      change.drop()
      report('Could Not Update the Title', error)
    }
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
        report('Could Not Reorder Conversations', error)
      })
  }

  /** Marks what the conversation shows as read, at once; the write follows the conversation's earlier ones. */
  function markRead(id: string): Promise<void> {
    const conversation = items.value.find((item) => item.id === id)
    if (
      !conversation ||
      conversation.load !== 'ready' ||
      !conversation.unread ||
      document.visibilityState !== 'visible'
    ) {
      return Promise.resolve()
    }
    const revision = conversation.revision
    const change = product.change(`conversation:${id}`, (state) =>
      withConversation(state, id, (record) => {
        const readRevision = Math.max(record.readRevision, revision)
        return { ...record, readRevision, unread: record.revision > readRevision }
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
        if (error instanceof ApiError && error.code === 'conversation_not_found') {
          return
        }
        report('Could Not Update Read Status', error)
      })
  }

  /**
   * Opens the conversation's tree again with the plugins the user has on
   * (`web-api.md` § A user's plugins); its socket is closed and reconnects,
   * and the summary stops offering the reload.
   */
  async function reloadPlugins(conversation: Conversation): Promise<void> {
    try {
      await apiRequest(
        `/conversations/${encodeURIComponent(conversation.id)}/reload`,
        { method: 'POST', signal: lifetime.signal },
      )
    } catch (error) {
      report('Could Not Reload the Conversation', error)
    }
  }

  /** Detaches a device the conversation's agents attached; a draft has none. */
  async function detachHost(
    conversation: Conversation,
    deviceId: string,
  ): Promise<void> {
    try {
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
      conversation.attachedHosts = conversation.attachedHosts.filter(
        (host) => host.deviceId !== deviceId,
      )
    } catch (error) {
      report('Could Not Detach the Device', error)
    }
  }

  /**
   * Makes one change of the conversation's model settings, which names only
   * the parts the user changed (`web-api.md` § Sidebar mutations and read
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
      rememberModel(conversation.model)
      return
    }
    // A record without a model takes the whole settings, as a first send
    // writes them.
    const recorded = product.snapshot?.conversations.some(
      (item) => item.id === conversation.id && item.model,
    )
    const settings = recorded ? null : applyModelChange(shownSettings(conversation), change)
    const signal = lifetime.signal
    try {
      const changed = await writes.run(conversation.id, async () => {
        signal.throwIfAborted()
        return patch(conversation.id, settings ? settingsPatch(settings) : change)
      })
      if (!changed) {
        return
      }
      signal.throwIfAborted()
      rememberModel(conversation.model)
      // A conversation opened without a usable model opens now with this
      // one, outside the queue: its opening waits for an archive or a
      // restore of it that may stand behind this write.
      const cached = cache.get(conversation.id)
      if (cached && !cached.runtime) {
        await reloadSession(conversation.id)
      }
    } catch (error) {
      report('Could Not Change the Model', error)
    }
  }

  /** The settings become the user's last choice, which new conversations start with. */
  function rememberModel(settings: ModelSettings): void {
    if (!settings.providerId || !settings.modelId) {
      return
    }
    preferences.update({ lastModel: { ...settings } }, true)
  }

  /**
   * Sends the draft the way the composer chose (`product.md` § Steer or
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
      (!conversation.draft.trim() && !conversation.attachmentIds.length)
    ) {
      return
    }
    if (conversation.persistence === 'draft') {
      conversation.persistence = 'pending'
    }
    const id = crypto.randomUUID()
    conversation.pendingSends = [...conversation.pendingSends, {
      id,
      way,
      text: conversation.draft,
      fileIds: messageFiles(conversation).map((file) => file.id),
      error: null,
    }]
    conversation.draft = ''
    conversation.attachmentIds = []
    await deliverInOrder(conversation, conversation.pendingSends.find((pending) => pending.id === id)!)
  }

  /**
   * Retry on a message whose delivery failed: it goes again with its id,
   * after the messages on their way, as a message. A steer the session
   * already holds is not sent again, and one it does not hold is answered
   * next, or queued while a turn runs.
   */
  async function retrySend(conversation: Conversation, id: string): Promise<void> {
    const pending = conversation.pendingSends.find((each) => each.id === id)
    if (conversation.archived || pending?.error == null) {
      return
    }
    pending.way = conversation.phase === 'idle' ? 'send' : 'queue'
    pending.error = null
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
   * message never goes without a file of it (`product.md` § Writing a
   * message): one that cannot be uploaded leaves it undelivered.
   */
  async function deliver(
    conversation: Conversation,
    pending: PendingSend,
    way: 'message' | 'steer',
  ): Promise<void> {
    const signal = lifetime.signal
    pending.error = null
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
      if (files.length !== pending.fileIds.length || files.length !== markCount(pending.text)) {
        throw new Error('A file of this message is no longer on this page. Copy its text, attach the file again and send it anew.')
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
      await (way === 'steer' ? runtime.steer(content, pending.id) : runtime.submit(content, pending.id))
      clearSubmission(conversation, pending.id)
    } catch (error) {
      // The page let the conversations go, as a sign-out does, or the message is no longer one to send.
      if (!signal.aborted && current()) {
        const wish = wishes.get(pending.id)
        // A message not delivered is no steer or queued message to act on:
        // one the user removed meanwhile is gone with its delivery.
        wishes.delete(pending.id)
        if (wish === 'remove') {
          dropSubmission(conversation, pending.id)
          stopRemoving(conversation, pending.id)
        } else {
          pending.error = error instanceof Error ? error.message : String(error)
        }
      }
    }
    if (!signal.aborted) {
      saveDrafts()
    }
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

  /** Takes a file on a paired device a pasted message names; null when the composer has it already. */
  function attachDeviceFile(conversation: Conversation, file: CopiedDeviceFile): ProductAttachment | null {
    if (remoteAttachmentError(file.path, file.host, conversation.files)) {
      return null
    }
    conversation.files.push({ ...composerRemoteAttachment({ host: file.host, path: file.path }), deviceId: file.deviceId })
    return conversation.files.at(-1) ?? null
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
    void draftSync.save(conversation).catch((error) => report('Could Not Clear the Draft', error))
    return true
  }

  /**
   * Keeps Remove or Send Now pressed on a message on its way as the user's
   * wish, when `id` is one; false for any other.
   */
  function keepWish(conversation: Conversation, id: string, wish: 'remove' | 'sendNow'): boolean {
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
  function carryOut(conversation: Conversation, id: string, wish: 'remove' | 'sendNow'): void {
    if (wish === 'remove') {
      carryOutRemoval(conversation, id)
    } else if (conversation.pendingSteers.some((steer) => steer.id === id)) {
      action(conversation, (runtime) => runtime.interruptPendingSteer(id))
    } else if (conversation.queue.some((message) => message.id === id)) {
      action(conversation, (runtime) => runtime.sendQueuedNow(id))
    }
  }

  /**
   * Remove on a steer or a queued message, or on a message on its way that
   * will be one (`product.md` § Steer or queue): its row goes at once, and
   * the request follows, once the session holds the message.
   */
  function removeWaiting(conversation: Conversation, id: string): void {
    if (conversation.removing.includes(id)) {
      return
    }
    conversation.removing = [...conversation.removing, id]
    if (!keepWish(conversation, id, 'remove')) {
      carryOutRemoval(conversation, id)
    }
  }

  /**
   * Removes the message `id` the session holds. A refusal, as when the agent
   * read the steer meanwhile, brings its row back where it was, which is
   * the transcript then, and a toast says why.
   */
  function carryOutRemoval(conversation: Conversation, id: string): void {
    void runtimeFor(conversation)
      .then((runtime) => runtime.removeWaiting(id))
      .catch((error) => report('Could Not Remove the Message', error))
      .finally(() => stopRemoving(conversation, id))
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

  function action(
    conversation: Conversation,
    operation: (runtime: ConversationRuntime) => Promise<void>,
  ): void {
    void runtimeFor(conversation)
      .then(operation)
      .catch((error) => {
        // A turn that ran and failed is already a record in the transcript.
        if (!isRecordedTurnFailure(error)) {
          report('The conversation did not accept the request.', error)
        }
      })
  }

  /**
   * Stops the conversation's turn. A Stop pressed while the message that
   * starts the turn is still on its way stops that turn once the session
   * holds the message, after the messages sent before the press; a message
   * that was not delivered started nothing.
   */
  async function stop(conversation: Conversation): Promise<void> {
    const starting = conversation.phase === 'idle'
      ? conversation.pendingSends.find((pending) => pending.way === 'send' && pending.error === null)?.id
      : undefined
    await deliveries.run(conversation.id, async () => {})
    if (starting !== undefined && conversation.pendingSends.some((pending) => pending.id === starting)) {
      return
    }
    action(conversation, (runtime) => runtime.abort())
  }

  function stopAll(): void {
    pendingChanges.value = []
    cache.clear()
    earlyReads.clear()
    restoringLocalDrafts.value = false
    madeHere.clear()
    deliveries = new SerialQueue()
    wishes.clear()
    draftSync.stop()
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
    reveal,
    history,
    activate,
    create,
    fork,
    reorder,
    rename,
    retitle,
    markRead,
    reloadList: () => product.reconnect(),
    reloadSession,
    pin: async (ids: string[], pinned: boolean) =>
      (await applyBatch(ids, { pinned }, true, pinned ? 'Pin' : 'Unpin')).success,
    archive,
    restore,
    remove,
    /**
     * Moves the conversation to run on `target` (`product.md` § Where a
     * conversation runs); with `notifyAgent`, the backend tells its agent.
     */
    switchTarget: (id: string, target: ConversationSummary['target'], notifyAgent = false) =>
      changeConversations(
        [id],
        async () => (await applyBatch([id], notifyAgent ? { target, notifyAgent } : { target }, false, 'Move')).success,
        false,
      ),
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
    send,
    retrySend,
    editVersion: (conversation: Conversation) =>
      cache.get(conversation.id)?.runtime?.transcriptVersion() ?? null,
    submitEdit: (conversation: Conversation) => submitMessageEdit(editHost(conversation)),
    regenerate: (conversation: Conversation, request: MessageEditRequest) =>
      regenerateMessage(editHost(conversation), request),
    addFiles,
    claimFile,
    attachDeviceFile,
    waitingForUploads,
    arrangeFiles,
    retryFile,
    saveDrafts,
    keepLastWords,
    /** Saves the drafts that wait for typing to pause: when the page is hidden, or, with `keepalive`, closes. */
    flushDrafts: (keepalive = false) => draftSync.flush(items.value, keepalive),
    restoreReplaced: (conversation: Conversation) =>
      void draftSync.act(conversation, 'restore').catch((error) => report('Could Not Restore the Draft', error)),
    dismissReplaced: (conversation: Conversation) =>
      void draftSync.act(conversation, 'dismiss').catch((error) => report('Could Not Dismiss the Draft', error)),
    initialize,
    stopAll,
    abortSubagents: (conversation: Conversation) =>
      action(conversation, (runtime) => runtime.abortSubagents()),
    abortSubagent: (conversation: Conversation, id: string) =>
      action(conversation, (runtime) => runtime.abortSubagent(id)),
    abortTerminal: (conversation: Conversation, id: string) =>
      action(conversation, (runtime) => runtime.abortTerminal(id)),
    start: (conversation: Conversation) =>
      action(conversation, (runtime) => runtime.resume()),
    stop,
    compact: (conversation: Conversation) =>
      action(conversation, (runtime) => runtime.compact()),
    // Send Now on a steer or a queued message still on its way waits for the
    // session to hold it; on any other, it goes at once. Remove takes the
    // row away at once in either case.
    removeQueued: removeWaiting,
    sendQueued: (conversation: Conversation, id: string) =>
      {
        if (!keepWish(conversation, id, 'sendNow')) {
          action(conversation, (runtime) => runtime.sendQueuedNow(id))
        }
      },
    removePendingSteer: removeWaiting,
    interruptWithSteer: (conversation: Conversation, id: string) =>
      {
        if (!keepWish(conversation, id, 'sendNow')) {
          action(conversation, (runtime) => runtime.interruptPendingSteer(id))
        }
      },
  }
})
