import { computed, reactive, ref, toRaw, watch } from 'vue'
import { defineStore } from 'pinia'
import { SerialQueue } from '@demicodes/utils'
import type { HeadlineText } from '@demicodes/web-ui/ui/ui-text'
import type { ClientContent } from '@demicodes/protocol'
import { ConversationCache, type CachedConversation } from '@demicodes/web-ui/agent/conversation-cache'
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
import { showArchived } from '@demicodes/web-ui/sidebar/archived-toast'
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
import {
  attachmentsReady,
  composerAttachmentFromFile,
  composerRemoteAttachment,
  isComposerFile,
} from '@demicodes/web-ui/agent/message-input/attachments'
import { connectConversationClient, takeConnection } from '@demicodes/web-ui/transport/conversation-socket'
import { loadDraft } from '../api/drafts'
import { ApiError, apiRequest, apiUrl, jsonBody, readResponse } from '../api/client'
import {
  attachedHostsSchema,
  batchAnswerSchema,
  createdConversationSchema,
  conversationUpdateSchema,
  transcriptSchema,
  type AttachedHosts,
  type ConversationBatch,
  type ConversationDraft,
  type ConversationPatch,
  type ConversationStatus,
  type ConversationSummary,
  type CreateConversation,
  type DraftFile,
  type ReadRequest,
  type SidebarReorder,
  type Transcript,
} from '../api/generated/web-api'
import { joinMessageContent } from '@demicodes/web-ui/agent/message-input/message-content'
import { createConversationUploads } from './uploads'
import { useResources } from '../state/resources'
import { useProduct } from '../state/product'
import { usePreferences } from '../state/preferences'
import { useSession } from '../auth/session'
import type { Conversation, ProductAttachment } from '../state/types'
import { applyConversationEvent, updateLiveStatus } from './activity'
import { createDraftSync, hasUnsavedDraft } from './draft-sync'
import { transcriptTerminals } from './terminals'
import {
  deleteDraft,
  readDraft,
  readLocalDrafts,
  takeLastWords,
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
  const pendingChanges = ref<string[]>([])
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
  /**
   * Counts the times New gave the user the draft already shown, whose
   * composer then takes the focus again.
   */
  const composerFocusRequests = ref(0)
  /**
   * The message a search result opened its conversation at, which the
   * conversation's page brings into view and marks once, then clears.
   */
  const reveal = ref<{ conversationId: string; blockId: string } | null>(null)
  /** The delivery of each conversation's sent message, while it is on its way. */
  const sending = new Map<string, Promise<void>>()
  let storageErrorReported = false

  // A failed operation is a toast; server state is never replaced by a message.
  function report(title: HeadlineText, error: unknown): void {
    if (error instanceof DOMException && error.name === 'AbortError') {
      return
    }
    if (!lifetime.signal.aborted) {
      reportError(title, error, { userVisible: true })
    }
  }

  function storageError(error: unknown): void {
    if (!storageErrorReported) {
      storageErrorReported = true
      reportError('Drafts remain on this page but could not be saved.', error, {
        userVisible: true,
      })
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

  /** Titles the user has set that the backend has not confirmed yet, by conversation. */
  const pendingTitles = new Map<string, string>()
  /** Conversations whose title request the backend has not acknowledged yet. */
  const pendingRetitles = new Set<string>()

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
      blocks: [],
      phase: 'idle',
      queue: [],
      pendingSteers: [],
      pendingCalls: [],
      model: record.model ? { ...record.model } : initialModelSettings(),
      lastError: null,
      draft: '',
      savedDraft: null,
      draftBase: null,
      draftShown: 0,
      files: [],
      attachmentIds: [],
      pendingSend: null,
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
      const local = previous.filter(
        (item) =>
          item.persistence !== 'synced' &&
          !snapshot.conversations.some((record) => record.id === item.id),
      )
      const next = snapshot.conversations.map((record) => {
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
        // A summary read before the rename reached the backend must not show the old title again.
        current.title = pendingTitles.get(current.id) ?? current.title
        // Nor may one read before the title request arrived offer Detect Title again.
        current.titleGenerating ||= pendingRetitles.has(current.id)
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
          cache.delete(current.id)
          current.load = 'loading'
        }
        return current
      })
      for (const item of local.toReversed()) {
        const following = previous
          .slice(previous.indexOf(item) + 1)
          .find((candidate) => next.some((entry) => entry.id === candidate.id))
        const index = following
          ? next.findIndex((entry) => entry.id === following.id)
          : next.length
        next.splice(index, 0, item)
      }
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
    const sending = new Set(conversation.pendingSend?.fileIds ?? [])
    const files = [
      ...(unsaved ? messageFiles(conversation) : []),
      ...conversation.files.filter((file) => sending.has(file.id)),
    ]
    return {
      messageEdit: conversation.messageEdit
        ? structuredClone(toRaw(conversation.messageEdit))
        : null,
      pendingSend: conversation.pendingSend
        ? structuredClone(toRaw(conversation.pendingSend))
        : null,
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
        !entry.draft.pendingSend &&
        !entry.draft.text.trim() &&
        !entry.draft.files.length
          ? deleteDraft(userId, entry.id)
          : writeDraft(userId, entry.id, entry.draft)
      void operation.catch((error) => {
        // A draft that did not reach storage counts as changed, so the next
        // save tries it again.
        savedDrafts.delete(entry.id)
        if (current === lifetime) {
          storageError(error)
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
      const drafts = (await readLocalDrafts(userId)).sort((a, b) =>
        (a.local?.conversation.createdAt ?? '').localeCompare(
          b.local?.conversation.createdAt ?? '',
        ),
      )
      signal.throwIfAborted()
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
        items.value.unshift(conversation)
        const restoredConversation = items.value.find(
          (item) => item.id === conversation.id,
        )!
        await restoreDraft(restoredConversation, signal, draft)
      }
    } catch (error) {
      if (!signal.aborted) {
        storageError(error)
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
        conversation.pendingSend = draft.pendingSend
        conversation.messageEdit = restoreMessageEdit(draft.messageEdit ?? null)
        if (conversation.pendingSend && !conversation.pendingSend.error) {
          conversation.pendingSend.error =
            'Delivery was interrupted. Retry to confirm it.'
        }
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
        // are those of the send not yet accepted.
        const sending = new Set(draft.pendingSend?.fileIds ?? [])
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
      storageError(error)
    }
    restored.add(conversation.id)
    // The page starts from what storage held: saving the restored draft
    // unchanged could only replace a newer one another tab saved since.
    changedDraft(conversation.id, persisted(conversation))
    for (const file of conversation.files) {
      if (isComposerFile(file) && !file.upload) {
        void uploadFile(file).catch((error) => report('Could Not Upload the Attachment', error))
      }
    }
  }

  /** A file of a draft this web browser kept, as the composer carries it. */
  function savedAttachment(file: SavedFile | Extract<ProductAttachment, { kind: 'reference' }>): ProductAttachment {
    if (file.kind === 'reference') {
      return file
    }
    return {
      ...(file.file ? composerAttachmentFromFile(file.file) : { src: file.upload ? blobPicture(file.upload) : undefined }),
      ...file,
      phase: file.upload ? 'ready' : 'uploading',
    }
  }

  /** Where an uploaded picture loads from, for a file whose bytes this web browser does not have. */
  function blobPicture(upload: { mediaType: string; sha256: string }): string | undefined {
    return upload.mediaType.startsWith('image/')
      ? apiUrl(`/blobs/${upload.sha256}?${new URLSearchParams({ type: upload.mediaType })}`)
      : undefined
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
      // refetching REST history; an open one is kept as it is.
      await cache.get(conversation.id)?.runtime?.connect()
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
   * The reads an opening needs that name only the conversation: its
   * transcript, its attached hosts and its draft, sent together
   * (`web-application.md` § Requests for one action).
   */
  interface OpeningReads {
    transcript: Promise<Transcript>
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
      transcript: apiRequest(`${path}/transcript`, { signal }).then((response) => readResponse(response, transcriptSchema)),
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
      const pick = composerModel(
        resources.providerInfos,
        resources.modelsFor(),
        conversation.model.providerId,
        conversation.model.modelId,
      )
      if (conversation.archived || pick.kind !== 'ready') {
        discardEarly()
        return
      }
      const runtime = new ConversationRuntime({
        state: conversation,
        // The first connection is the one made beside the reads.
        connect: (attempt) => {
          const made = early
          early = null
          return made ? takeConnection(made, attempt) : connectConversationClient(streamUrl(conversation.id), attempt)
        },
        onEvent: (next) => {
          applyConversationEvent(conversation, next)
          reconcileSubmission(conversation)
        },
      })
      entry.runtime = runtime
      await runtime.connect()
    } catch (error) {
      discardEarly()
      if (!signal.aborted) {
        conversation.load = 'failed'
        conversation.lastError =
          error instanceof Error ? error.message : String(error)
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
    const [transcript] = await Promise.all([reads.transcript, draftRead])
    signal.throwIfAborted()
    conversation.blocks = transcript.blocks
    conversation.failures = transcript.failures ?? {}
    reconcileSubmission(conversation)
    conversation.terminals = transcriptTerminals(transcript.blocks)
    conversation.subagents = transcript.subagents.map(({ subagent, blocks, failures }) => ({
      pendingCalls: [],
      id: subagent.subagentId,
      name: subagent.description,
      phase: subagent.phase,
      startedAt: subagent.startedAt,
      endedAt: subagent.endedAt ?? undefined,
      blocks,
      failures: failures ?? {},
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

  async function patch(id: string, changes: ConversationPatch): Promise<boolean> {
    const signal = lifetime.signal
    const sentAt = product.sent()
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
    // The answer is the record as the patch left it, which shows at once.
    product.answered(sentAt, { type: 'conversation', conversation: result.conversation })
    const failed = result.results.flatMap((field) => field.status === 'failed' ? [field] : [])
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

  function batch(
    ids: string[],
    changes: Parameters<typeof applyBatch>[1],
  ): Promise<boolean> {
    return changeConversations(ids, () => applyBatch(ids, changes), false)
  }

  async function applyBatch(
    ids: string[],
    changes: Pick<ConversationPatch, 'pinned' | 'archived' | 'target' | 'notifyAgent'>,
  ): Promise<boolean> {
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
    ids = ids.filter(
      (id) =>
        items.value.find((item) => item.id === id)?.persistence === 'synced',
    )
    saveDrafts()
    if (!ids.length) {
      return true
    }
    const signal = lifetime.signal
    try {
      return await writes.run(async () => {
        signal.throwIfAborted()
        let success = true
        for (let offset = 0; offset < ids.length; offset += 100) {
          const sentAt = product.sent()
          const response = await apiRequest('/conversations/batch', {
            method: 'POST',
            signal,
            ...jsonBody({
              items: ids.slice(offset, offset + 100).map((id) => ({
                id,
                patch: changes,
              })),
            } satisfies ConversationBatch),
          })
          const result = await readResponse(response, batchAnswerSchema)
          signal.throwIfAborted()
          for (const item of result.results) {
            if (item.status === 'updated') {
              product.answered(sentAt, { type: 'conversation', conversation: item.conversation })
            }
          }
          const titleOf = (id: string) =>
            items.value.find((conversation) => conversation.id === id)?.title ?? id
          const failures = result.results.flatMap((item) =>
            item.status === 'refused'
              ? [`${titleOf(item.id)}: ${item.message}`]
              : item.results.flatMap((field) =>
                  field.status === 'failed' ? [`${titleOf(item.id)}: ${field.message}`] : [],
                ),
          )
          if (failures.length) {
            success = false
            reportError('Some conversations were not updated.', failures.join('\n'), {
              userVisible: true,
              expected: true,
            })
          }
        }
        return success
      })
    } catch (error) {
      report('Could Not Update Conversations', error)
      return false
    }
  }

  /**
   * Archives the conversations and says so in a toast whose Undo restores
   * the ones it archived (`product.md` § Conversations and projects).
   * Answers whether every one was archived.
   */
  async function archive(ids: string[]): Promise<boolean> {
    const done = await batch(ids, { archived: true })
    const archived = ids.filter((id) => items.value.find((item) => item.id === id)?.archived)
    if (archived.length) {
      showArchived(archived.length, () => void restore(archived))
    }
    return done
  }

  /**
   * Deletes the conversations for good, once the user confirmed it
   * (`product.md` § Conversations and projects): one the backend has no
   * record of leaves this page, and each other one leaves the backend and
   * every page. What this browser keeps of their drafts goes too. A refusal
   * is a toast. Answers whether every one was deleted.
   */
  function remove(ids: string[]): Promise<boolean> {
    return changeConversations(ids, () => applyRemove(ids), false)
  }

  async function applyRemove(ids: string[]): Promise<boolean> {
    const userId = session.user?.id ?? product.snapshot?.user.id
    const forget = (id: string) => {
      restored.delete(id)
      savedDrafts.delete(id)
      if (userId) {
        void deleteDraft(userId, id).catch(storageError)
      }
    }
    const synced = ids.filter((id) => items.value.find((item) => item.id === id)?.persistence === 'synced')
    const local = ids.filter((id) => !synced.includes(id))
    items.value = items.value.filter((item) => !local.includes(item.id))
    local.forEach(forget)
    const signal = lifetime.signal
    try {
      return await writes.run(async () => {
        const failures: string[] = []
        for (const id of synced) {
          signal.throwIfAborted()
          const title = items.value.find((item) => item.id === id)?.title ?? id
          const sentAt = product.sent()
          try {
            await apiRequest(`/conversations/${encodeURIComponent(id)}`, { method: 'DELETE', signal })
          } catch (error) {
            if (signal.aborted) {
              throw error
            }
            failures.push(`${title}: ${error instanceof Error ? error.message : String(error)}`)
            continue
          }
          // The deletion shows at once; the channel brings it to every other page.
          product.answered(sentAt, { type: 'conversation_deleted', id })
          forget(id)
        }
        if (failures.length) {
          reportError(
            failures.length === 1 && synced.length === 1
              ? 'Could Not Delete the Conversation'
              : 'Some conversations were not deleted.',
            failures.join('\n'),
            { userVisible: true, expected: true },
          )
        }
        return failures.length === 0
      })
    } catch (error) {
      report('Could Not Delete Conversations', error)
      return false
    }
  }

  /** Brings archived conversations back; answers whether every one came back. */
  function restore(ids: string[]): Promise<boolean> {
    return batch(ids, { archived: false })
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
    // The new title shows at once and stays while the write is on its way;
    // a write that fails or is refused gives the stored title back.
    if (conversation) {
      conversation.title = title
    }
    pendingTitles.set(id, title)
    const signal = lifetime.signal
    void writes
      .run(() => {
        signal.throwIfAborted()
        return patch(id, { title })
      })
      .catch((error) => {
        report('Could Not Rename the Conversation', error)
        return false
      })
      .then((renamed) => {
        if (pendingTitles.get(id) !== title) {
          return
        }
        pendingTitles.delete(id)
        const stored = product.snapshot?.conversations.find((item) => item.id === id)
        if (!renamed && conversation && stored) {
          conversation.title = stored.title
        }
      })
  }

  /** Asks for a title from the conversation as it stands (`product.md` § Conversation titles). */
  async function retitle(conversation: Conversation): Promise<void> {
    if (conversation.titleGenerating || conversation.persistence !== 'synced') {
      return
    }
    conversation.titleGenerating = true
    pendingRetitles.add(conversation.id)
    try {
      // The backend asks the model the conversation's record holds.
      await apiRequest(
        `/conversations/${encodeURIComponent(conversation.id)}/title`,
        { method: 'POST', signal: lifetime.signal },
      )
      // From here the conversation's summary says whether the request is
      // still running.
      pendingRetitles.delete(conversation.id)
    } catch (error) {
      pendingRetitles.delete(conversation.id)
      conversation.titleGenerating = false
      report('Could Not Update the Title', error)
    }
  }

  async function reorder(id: string, beforeId: string | null): Promise<void> {
    const conversation = items.value.find((item) => item.id === id)
    if (conversation && conversation.persistence !== 'synced') {
      const reordered = items.value.filter((item) => item.id !== id)
      const index = reordered.findIndex((item) => item.id === beforeId)
      reordered.splice(index < 0 ? reordered.length : index, 0, conversation)
      items.value = reordered
      return
    }
    if (beforeId) {
      const index = items.value.findIndex((item) => item.id === beforeId)
      beforeId =
        items.value.slice(index).find((item) => item.persistence === 'synced')
          ?.id ?? null
    }
    try {
      await apiRequest('/sidebar/reorder', {
        method: 'POST',
        signal: lifetime.signal,
        ...jsonBody({
          kind: 'conversation',
          id,
          beforeId,
        } satisfies SidebarReorder),
      })
    } catch (error) {
      report('Could Not Reorder Conversations', error)
    }
  }

  async function markRead(id: string): Promise<void> {
    const conversation = items.value.find((item) => item.id === id)
    if (
      !conversation ||
      conversation.load !== 'ready' ||
      !conversation.unread ||
      document.visibilityState !== 'visible'
    ) {
      return
    }
    const revision = conversation.revision
    try {
      await apiRequest(`/conversations/${encodeURIComponent(id)}/read`, {
        method: 'POST',
        signal: lifetime.signal,
        ...jsonBody({ revision } satisfies ReadRequest),
      })
      conversation.readRevision = Math.max(conversation.readRevision, revision)
      conversation.unread = conversation.revision > conversation.readRevision
    } catch (error) {
      // A conversation deleted meanwhile, by this page or another, has
      // nothing left to acknowledge.
      if (error instanceof ApiError && error.code === 'conversation_not_found') {
        return
      }
      report('Could Not Update Read Status', error)
    }
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
      await writes.run(async () => {
        signal.throwIfAborted()
        const body = settings ? settingsPatch(settings) : change
        if (!(await patch(conversation.id, body))) {
          return
        }
        signal.throwIfAborted()
        rememberModel(conversation.model)
        // A conversation opened without a usable model opens now with this one.
        const cached = cache.get(conversation.id)
        if (cached && !cached.runtime) {
          await reloadSession(conversation.id)
        }
      })
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

  async function send(conversation: Conversation): Promise<void> {
    if (
      conversation.archived ||
      conversation.pendingSend?.error === null ||
      (!conversation.pendingSend && !attachmentsReady(messageFiles(conversation))) ||
      (!conversation.pendingSend &&
        !conversation.draft.trim() &&
        !conversation.attachmentIds.length)
    ) {
      return
    }
    if (conversation.persistence === 'draft') {
      conversation.persistence = 'pending'
    }
    if (!conversation.pendingSend) {
      conversation.pendingSend = {
        id: crypto.randomUUID(),
        text: conversation.draft,
        fileIds: messageFiles(conversation).map((file) => file.id),
        error: null,
      }
      conversation.draft = ''
      conversation.attachmentIds = []
    }
    const pending = conversation.pendingSend
    const delivery = deliver(conversation, pending)
    sending.set(conversation.id, delivery)
    try {
      await delivery
    } finally {
      if (sending.get(conversation.id) === delivery) {
        sending.delete(conversation.id)
      }
    }
  }

  /**
   * Delivers the message the conversation holds as sent: makes its record
   * first when it has none, then gives the message to the session with its
   * id, which takes it once.
   */
  async function deliver(
    conversation: Conversation,
    pending: NonNullable<Conversation['pendingSend']>,
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
    const current = () => conversation.pendingSend?.id === pending.id
    try {
      await persistConversation(conversation)
      const references = files.map((file): ClientContent => {
        if (!isComposerFile(file)) {
          return { type: 'remote_file', deviceId: file.deviceId, path: file.path }
        }
        if (!file.upload) {
          throw new Error(`${file.name} has not finished uploading.`)
        }
        return { type: 'upload', ref: file.upload.id, fileName: file.name }
      })
      // Each file where its capsule stands in the text.
      const content = joinMessageContent(pending.text, references.map((reference) => [reference]))
      await (await runtimeFor(conversation)).submit(content, pending.id)
      clearSubmission(conversation, pending.id)
    } catch (error) {
      // The page let the conversations go, as a sign-out does, or the message is no longer the one to send.
      if (!signal.aborted && current()) {
        pending.error = error instanceof Error ? error.message : String(error)
      }
    }
    if (!signal.aborted) {
      saveDrafts()
    }
  }

  /** The files the message has, in the order of its capsules; the rest wait for an undo. */
  function messageFiles(conversation: Conversation): Conversation['files'] {
    return conversation.attachmentIds.flatMap((id) =>
      conversation.files.filter((file) => file.id === id),
    )
  }

  function clearSubmission(conversation: Conversation, id: string): void {
    const pending = conversation.pendingSend
    if (pending?.id !== id) {
      return
    }
    conversation.pendingSend = null
    for (const fileId of pending.fileIds) {
      removeFile(conversation, fileId)
    }
    // The message is gone: what its composer still held for an undo goes with it.
    releaseSpare(conversation)
    saveDrafts()
    // The backend accepted the message: the draft it was written in goes too,
    // everywhere, and a change another page made meanwhile stays to restore.
    void draftSync.save(conversation).catch((error) => report('Could Not Clear the Draft', error))
  }

  function reconcileSubmission(conversation: Conversation): void {
    const id = conversation.pendingSend?.id
    if (id && hasAcceptedSubmission(conversation, id)) {
      clearSubmission(conversation, id)
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
   * holds the message; a message that was not delivered started nothing.
   */
  async function stop(conversation: Conversation): Promise<void> {
    await sending.get(conversation.id)
    if (conversation.pendingSend) {
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
    sending.clear()
    draftSync.stop()
    lifetime.abort()
    lifetime = new AbortController()
    uploads.dispose(items.value)
    items.value = []
    deleted.clear()
    restored.clear()
    savedDrafts.clear()
    storageErrorReported = false
  }

  return {
    items,
    listed,
    deleted,
    listStatus,
    composerFocusRequests,
    reveal,
    activate,
    create,
    fork,
    reorder,
    rename,
    retitle,
    markRead,
    reloadList: () => product.reconnect(),
    reloadSession,
    pin: (ids: string[], pinned: boolean) => batch(ids, { pinned }),
    archive,
    restore,
    remove,
    /**
     * Moves the conversation to run on `target` (`product.md` § Where a
     * conversation runs); with `notifyAgent`, the backend tells its agent.
     */
    switchTarget: (id: string, target: ConversationSummary['target'], notifyAgent = false) =>
      batch([id], notifyAgent ? { target, notifyAgent } : { target }),
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
    editVersion: (conversation: Conversation) =>
      cache.get(conversation.id)?.runtime?.transcriptVersion() ?? null,
    submitEdit: (conversation: Conversation) => submitMessageEdit(editHost(conversation)),
    regenerate: (conversation: Conversation, request: MessageEditRequest) =>
      regenerateMessage(editHost(conversation), request),
    addFiles,
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
    removeQueued: (conversation: Conversation, id: string) =>
      action(conversation, (runtime) => runtime.dequeueMessage(id)),
    sendQueued: (conversation: Conversation, id: string) =>
      action(conversation, (runtime) => runtime.sendQueuedNow(id)),
    removePendingSteer: (conversation: Conversation, id: string) =>
      action(conversation, (runtime) => runtime.deletePendingSteer(id)),
    interruptWithSteer: (conversation: Conversation, id: string) =>
      action(conversation, (runtime) => runtime.interruptPendingSteer(id)),
  }
})
