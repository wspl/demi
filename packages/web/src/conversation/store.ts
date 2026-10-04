import { computed, ref, toRaw, watch } from 'vue'
import { defineStore } from 'pinia'
import { SerialQueue } from '@demicodes/utils'
import type { HeadlineText } from '@demicodes/web-ui/ui/ui-text'
import type { ClientContent } from '@demicodes/protocol'
import { ConversationCache, type CachedConversation } from '@demicodes/web-ui/agent/conversation-cache'
import { ConversationRuntime, isRecordedTurnFailure } from '@demicodes/web-ui/agent/conversation-runtime'
import { restoreMessageEdit, sentEditRequest, submitMessageEdit } from '@demicodes/web-ui/agent/message-editing'
import { reportError } from '@demicodes/web-ui/infra/errors'
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
import { connectConversationClient } from '@demicodes/web-ui/transport/conversation-socket'
import { apiRequest, apiUrl, jsonBody, readResponse } from '../api/client'
import {
  attachedHostsSchema,
  batchAnswerSchema,
  conversationAnswerSchema,
  conversationUpdateSchema,
  transcriptSchema,
  type AttachHost,
  type ConversationBatch,
  type ConversationDraft,
  type ConversationPatch,
  type ConversationStatus,
  type ConversationSummary,
  type CreateConversation,
  type DraftFile,
  type ReadRequest,
  type RenameHost,
  type SidebarReorder,
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

export const useConversations = defineStore('conversations', () => {
  const product = useProduct()
  const preferences = usePreferences()
  const resources = useResources()
  const session = useSession()
  const items = ref<Conversation[]>([])
  const pendingChanges = ref<string[]>([])
  const listStatus = computed(() => product.load)
  const writes = new SerialQueue()
  const restored = new Set<string>()
  /** Each conversation's draft as this page last saved or restored it, in the shape `changedDraft` compares. */
  const savedDrafts = new Map<string, string>()
  const uploads = createConversationUploads(saveDrafts, (error) =>
    report('Could not upload the attachment', error),
  )
  const { uploadFile, addFiles, removeFile, releaseSpare, arrangeFiles, retryFile } = uploads
  const draftSync = createDraftSync({
    apply: showDraft,
    report,
  })
  let lifetime = new AbortController()
  const cache = new ConversationCache()
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
      reportError('Drafts remain in this page but could not be saved', error, {
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
      | 'contextVersion'
      | 'revision'
      | 'readRevision'
      | 'unread'
      | 'cwd'
      | 'titleCurrent'
      | 'titleGenerating'
      | 'pluginsChanged'
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
      contextVersion: record.contextVersion,
      cwd: record.cwd,
      revision: record.revision,
      readRevision: record.readRevision,
      unread: record.unread,
      titleCurrent: record.titleCurrent,
      titleGenerating: record.titleGenerating,
      pluginsChanged: record.pluginsChanged,
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
      model: record.model ? { ...record.model } : initialModelSettings(),
      lastError: null,
      draft: '',
      savedDraft: null,
      draftBase: null,
      draftShown: 0,
      files: [],
      attachmentIds: [],
      submission: 'idle',
      pendingSend: null,
      messageEdit: null,
      scroll: null,
      attachedHosts: [],
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
        const revisionChanged = current.revision !== record.revision
        const contextChanged = current.contextVersion !== record.contextVersion
        const archiveChanged = current.archived !== record.archived
        Object.assign(current, metadata(record))
        followRecordModel(current, record)
        // A summary read before the rename reached the backend must not show the old title again.
        current.title = pendingTitles.get(current.id) ?? current.title
        // Nor may one read before the title request arrived stop its button spinning.
        current.titleGenerating ||= pendingRetitles.has(current.id)
        const cached = cache.get(current.id)
        if (!cached?.runtime?.connected) {
          current.status = summaryStatus(record.status)
        }
        // Another page saved the draft since this one read it.
        if (current.savedDraft && record.draftRevision > current.savedDraft.revision) {
          void draftSync.read(current).catch((error) => report('Could not read the draft', error))
        }
        if (
          revisionChanged &&
          cached &&
          !contextChanged &&
          !archiveChanged
        ) {
          void loadHosts(current, cached.controller.signal).catch((error) => report('Could not load the conversation hosts', error))
        }
        if (contextChanged || archiveChanged) {
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
      const active = next.find((item) => item.id === product.activeConversationId)
      if (active && active.load !== 'failed' && !cache.get(active.id)) {
        void activate(active.id)
      }
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
              hosts: conversation.attachedHosts.map(
                ({ deviceId, name, cwd }) => ({ deviceId, name, cwd }),
              ),
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
          pluginRevisions: [],
          workingTreeRevision: 0,
          status: 'idle',
          contextVersion: 0,
          revision: 0,
          readRevision: 0,
          unread: false,
        })
        conversation.persistence = draft.local.phase
        conversation.attachedHosts = draft.local.hosts.map((host) => ({
          ...host,
          online: resources.devices.some(
            (device) => device.id === host.deviceId && device.online,
          ),
        }))
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
    await restoreLocalDrafts()
    if (!signal.aborted) {
      product.start()
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
            'Sending was interrupted. Retry to confirm delivery.'
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
        void uploadFile(file).catch((error) => report('Could not upload the attachment', error))
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

  async function activate(id: string | null): Promise<void> {
    const previous = items.value.find(
      (item) => item.id === product.activeConversationId,
    )
    if (
      previous &&
      previous.id !== id &&
      previous.persistence === 'draft' &&
      !previous.draft.trim() &&
      !previous.attachmentIds.length
    ) {
      saveDrafts()
      items.value = items.value.filter((item) => item !== previous)
      restored.delete(previous.id)
      savedDrafts.delete(previous.id)
      cache.delete(previous.id)
    }
    product.activeConversationId = id
    const conversation = items.value.find((item) => item.id === id)
    if (!conversation) {
      return
    }
    if (!cache.get(conversation.id)) {
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

  async function loadConversation(
    conversation: Conversation,
    entry: CachedConversation,
  ): Promise<void> {
    const { controller } = entry
    try {
      await restoreDraft(conversation, controller.signal)
      if (conversation.persistence !== 'synced') {
        conversation.load = 'ready'
        return
      }
      // Share global model discovery, but render history before it completes.
      // Model load errors belong to the composer, not transcript restoration.
      const modelsLoaded = product.loadModels().catch(() => {})
      controller.signal.throwIfAborted()
      // Read beside the history; a submission it confirms clears the draft after it.
      const draftRead = draftSync.read(conversation)
      // Its failure fails the load below, unless an earlier one did.
      draftRead.catch(() => {})
      await loadHosts(conversation, controller.signal)
      const response = await apiRequest(
        `/conversations/${encodeURIComponent(conversation.id)}/transcript`,
        { signal: controller.signal },
      )
      const transcript = await readResponse(response, transcriptSchema)
      await draftRead
      controller.signal.throwIfAborted()
      conversation.blocks = transcript.blocks
      conversation.failures = transcript.failures ?? {}
      reconcileSubmission(conversation)
      conversation.terminals = transcriptTerminals(transcript.blocks)
      conversation.subagents = transcript.subagents.map(({ subagent, blocks, failures }) => ({
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
      await modelsLoaded
      controller.signal.throwIfAborted()
      const pick = composerModel(
        resources.providerInfos,
        resources.modelsFor(),
        conversation.model.providerId,
        conversation.model.modelId,
      )
      if (conversation.archived || pick.kind !== 'ready') {
        return
      }
      const runtime = new ConversationRuntime({
        state: conversation,
        connect: (signal) => {
          const url = new URL(
            `/api/conversations/${encodeURIComponent(conversation.id)}/stream`,
            window.location.href,
          )
          url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:'
          return connectConversationClient(url.toString(), signal)
        },
        onEvent: (next) => {
          applyConversationEvent(conversation, next)
          reconcileSubmission(conversation)
        },
      })
      entry.runtime = runtime
      await runtime.connect()
    } catch (error) {
      if (!controller.signal.aborted) {
        conversation.load = 'failed'
        conversation.lastError =
          error instanceof Error ? error.message : String(error)
      }
      throw error
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

  async function loadHosts(
    conversation: Conversation,
    signal = lifetime.signal,
  ): Promise<void> {
    const response = await apiRequest(
      `/conversations/${encodeURIComponent(conversation.id)}/hosts`,
      { signal },
    )
    const result = await readResponse(response, attachedHostsSchema)
    signal.throwIfAborted()
    conversation.attachedHosts = result.hosts
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
        'Some fields were not updated',
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
    changes: Pick<ConversationPatch, 'pinned' | 'archived' | 'target'>,
  ): Promise<boolean> {
    for (const conversation of items.value) {
      if (
        ids.includes(conversation.id) &&
        conversation.persistence !== 'synced'
      ) {
        Object.assign(conversation, changes)
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
            reportError('Some conversations were not updated', failures.join('\n'), {
              userVisible: true,
              expected: true,
            })
          }
        }
        return success
      })
    } catch (error) {
      report('Could not update conversations', error)
      return false
    }
  }

  function create(projectId: string | null = null): string {
    const empty = items.value.find(
      (item) =>
        item.persistence === 'draft' &&
        !item.archived &&
        item.projectId === projectId &&
        !item.draft.trim() &&
        !item.attachmentIds.length,
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
      pluginRevisions: [],
      workingTreeRevision: 0,
      createdAt: now,
      updatedAt: now,
      model: null,
      status: 'idle',
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

  async function persistConversation(
    conversation: Conversation,
  ): Promise<void> {
    if (conversation.persistence === 'synced') {
      return
    }
    const signal = lifetime.signal
    const sentAt = product.sent()
    const response = await apiRequest('/conversations', {
      method: 'POST',
      signal,
      ...jsonBody({ id: conversation.id } satisfies CreateConversation),
    })
    const result = await readResponse(response, conversationAnswerSchema)
    signal.throwIfAborted()
    product.answered(sentAt, { type: 'conversation', conversation: result.conversation })
    // The first send writes the conversation's model settings to its record:
    // for a part its model no longer offers, the model's first effort and the
    // vendor's default tier.
    const shown = shownSettings(conversation)
    const listed = lookupSelectedModel(resources.modelsFor(), shown.providerId, shown.modelId)
    const settings = listed ? offeredSettings(shown, listed.model) : null
    if (
      !(await patch(result.conversation.id, {
        title: conversation.title,
        target: conversation.target,
        pinned: conversation.pinned,
        ...(settings ? settingsPatch(settings) : {}),
      }))
    ) {
      throw new Error(
        'Could not configure the conversation. Try sending again.',
      )
    }
    for (const host of conversation.attachedHosts) {
      await apiRequest(
        `/conversations/${encodeURIComponent(conversation.id)}/hosts`,
        {
          method: 'POST',
          signal,
          ...jsonBody({ deviceId: host.deviceId } satisfies AttachHost),
        },
      )
      await apiRequest(
        `/conversations/${encodeURIComponent(
          conversation.id,
        )}/hosts/${encodeURIComponent(host.deviceId)}`,
        {
          method: 'PATCH',
          signal,
          ...jsonBody({ name: host.name } satisfies RenameHost),
        },
      )
    }
    signal.throwIfAborted()
    conversation.persistence = 'synced'
    cache.delete(conversation.id)
    const stored = product.snapshot?.conversations.find(
      (item) => item.id === conversation.id,
    )
    if (stored) {
      Object.assign(conversation, metadata(stored))
      followRecordModel(conversation, stored)
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
        report('Could not rename the conversation', error)
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
      report('Could not update the title', error)
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
      report('Could not reorder conversations', error)
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
      report('Could not update read status', error)
    }
  }

  async function attachHost(
    conversation: Conversation,
    deviceId: string,
  ): Promise<void> {
    if (conversation.persistence !== 'synced') {
      const device = resources.devices.find((item) => item.id === deviceId)
      if (
        device &&
        !conversation.attachedHosts.some((host) => host.deviceId === deviceId)
      ) {
        conversation.attachedHosts.push({
          deviceId,
          name: device.name,
          cwd: null,
          online: device.online,
        })
        saveDrafts()
      }
      return
    }
    try {
      await apiRequest(
        `/conversations/${encodeURIComponent(conversation.id)}/hosts`,
        {
          method: 'POST',
          signal: lifetime.signal,
          ...jsonBody({ deviceId } satisfies AttachHost),
        },
      )
      await loadHosts(conversation)
    } catch (error) {
      report('Could not attach the device', error)
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
      report('Could not reload the conversation', error)
    }
  }

  async function detachHost(
    conversation: Conversation,
    deviceId: string,
  ): Promise<void> {
    if (conversation.persistence !== 'synced') {
      conversation.attachedHosts = conversation.attachedHosts.filter(
        (host) => host.deviceId !== deviceId,
      )
      saveDrafts()
      return
    }
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
      await loadHosts(conversation)
    } catch (error) {
      report('Could not detach the device', error)
    }
  }

  async function renameHost(
    conversation: Conversation,
    deviceId: string,
    name: string,
  ): Promise<void> {
    if (conversation.persistence !== 'synced') {
      const host = conversation.attachedHosts.find(
        (item) => item.deviceId === deviceId,
      )
      if (host) {
        host.name = name
        saveDrafts()
      }
      return
    }
    try {
      await apiRequest(
        `/conversations/${encodeURIComponent(
          conversation.id,
        )}/hosts/${encodeURIComponent(deviceId)}`,
        {
          method: 'PATCH',
          signal: lifetime.signal,
          ...jsonBody({ name } satisfies RenameHost),
        },
      )
      await loadHosts(conversation)
    } catch (error) {
      report('Could not rename the project', error)
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
      report('Could not change the model', error)
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
      conversation.submission === 'sending' ||
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
    const signal = lifetime.signal
    pending.error = null
    // The files in the order of their marks in the text.
    const files = pending.fileIds.flatMap((id) =>
      conversation.files.filter((file) => file.id === id),
    )
    conversation.submission = 'sending'
    saveDrafts()
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
      if (!signal.aborted && conversation.pendingSend?.id === pending.id) {
        pending.error = error instanceof Error ? error.message : String(error)
      }
    } finally {
      conversation.submission = 'idle'
      if (!signal.aborted) {
        saveDrafts()
      }
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
    void draftSync.save(conversation).catch((error) => report('Could not clear the draft', error))
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
          report('The conversation did not accept the request', error)
        }
      })
  }

  function stopAll(): void {
    pendingChanges.value = []
    cache.clear()
    draftSync.stop()
    lifetime.abort()
    lifetime = new AbortController()
    uploads.dispose(items.value)
    items.value = []
    restored.clear()
    savedDrafts.clear()
    storageErrorReported = false
  }

  return {
    items,
    listStatus,
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
    archive: (ids: string[], archived = true) => batch(ids, { archived }),
    move: (ids: string[], projectId: string | null) =>
      batch(ids, {
        target: projectId
          ? {
              kind: 'workspace',
              workspaceId: projectId,
            }
          : { kind: 'cloud' },
      }),
    switchTarget: (id: string, target: ConversationSummary['target']) =>
      batch([id], { target }),
    pendingChanges,
    attachHost: (conversation: Conversation, deviceId: string) =>
      changeConversations(
        [conversation.id],
        () => attachHost(conversation, deviceId),
        undefined,
      ),
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
    renameHost: (conversation: Conversation, deviceId: string, name: string) =>
      changeConversations(
        [conversation.id],
        () => renameHost(conversation, deviceId, name),
        undefined,
      ),
    changeModel,
    send,
    editVersion: (conversation: Conversation) =>
      cache.get(conversation.id)?.runtime?.transcriptVersion() ?? null,
    submitEdit: (conversation: Conversation) => submitMessageEdit({
      get: () => conversation.messageEdit,
      set: (state) => { conversation.messageEdit = state },
      send: async (request) => {
        const edit = sentEditRequest(request)
        const runtime = await runtimeFor(conversation)
        await runtime.editAndSend(edit)
      },
    }),
    addFiles,
    arrangeFiles,
    retryFile,
    saveDrafts,
    keepLastWords,
    /** Saves the drafts that wait for typing to pause: when the page is hidden, or, with `keepalive`, closes. */
    flushDrafts: (keepalive = false) => draftSync.flush(items.value, keepalive),
    restoreReplaced: (conversation: Conversation) =>
      void draftSync.act(conversation, 'restore').catch((error) => report('Could not restore the draft', error)),
    dismissReplaced: (conversation: Conversation) =>
      void draftSync.act(conversation, 'dismiss').catch((error) => report('Could not dismiss the draft', error)),
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
    stop: (conversation: Conversation) =>
      action(conversation, (runtime) => runtime.abort()),
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
