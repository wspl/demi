import { computed, ref, toRaw, watch } from 'vue'
import { defineStore } from 'pinia'
import { z } from 'zod'
import { SerialQueue } from '@demicodes/utils'
import { type ThinkingConfig, type UserContentBlock } from '@demicodes/core'
import { ConversationCache, type CachedConversation } from '@demicodes/web-ui/agent/conversation-cache'
import { ConversationRuntime, isRecordedTurnFailure } from '@demicodes/web-ui/agent/conversation-runtime'
import { restoreMessageEdit, submitMessageEdit } from '@demicodes/web-ui/agent/message-editing'
import { reportError } from '@demicodes/web-ui/infra/errors'
import { loadEditContent } from '../api/message-editing'
import { forkConversation } from '../api/message-fork'
import type { MessageForkRequest } from '@demicodes/web-ui/agent/message-fork'
import { composerModel } from '@demicodes/web-ui/agent/model-selection'
import { thinkingConfigToEffort } from '@demicodes/web-ui/agent/reasoning'
import { hasAcceptedSubmission } from '@demicodes/web-ui/agent/submission'
import {
  attachmentsReady,
  composerAttachmentFromFile,
  isComposerFile,
  attachTextSnippet,
} from '@demicodes/web-ui/agent/message-input/attachments'
import { connectAgentClient } from '@demicodes/web-ui/transport/agent-socket'
import { type ProviderSelection } from '@demicodes/web-ui/transport/protocol'
import { apiRequest, jsonBody, readResponse } from '../api/client'
import {
  conversationBatchSchema,
  conversationRecordSchema,
  conversationUpdateSchema,
  hostsSchema,
  type BackendConversation,
} from '@demicodes/product-contracts'
import {
  contentReference,
  decodeServerFrame,
  encodeClientFrame,
  transcriptSchema,
} from '@demicodes/product-contracts'
import { createConversationUploads } from './uploads'
import { useResources } from '../state/resources'
import { useProduct } from '../state/product'
import { useSession } from '../auth/session'
import type { Conversation } from '../state/types'
import { applyConversationEvent, updateLiveStatus } from './activity'
import { transcriptTerminals } from './terminals'
import {
  deleteDraft,
  readDraft,
  DraftDataError,
  readLocalDrafts,
  writeDraft,
  type SavedDraft,
} from './drafts'

export const useConversations = defineStore('conversations', () => {
  const product = useProduct()
  const resources = useResources()
  const session = useSession()
  const items = ref<Conversation[]>([])
  const pendingChanges = ref<string[]>([])
  const listStatus = computed(() => product.load)
  const writes = new SerialQueue()
  const restored = new Set<string>()
  const uploads = createConversationUploads(saveDrafts, (error) =>
    report('Could not upload the attachment', error),
  )
  const { uploadFile, addFiles, removeFile } = uploads
  let lifetime = new AbortController()
  const cache = new ConversationCache()
  let storageErrorReported = false

  // A failed operation is a toast; server state is never replaced by a message.
  function report(title: string, error: unknown): void {
    if (error instanceof DOMException && error.name === 'AbortError') {
      return
    }
    if (!lifetime.signal.aborted) {
      reportError(title, error, { userVisible: true })
    }
  }

  // The product store tells a refresh failure itself, once per outage.
  function refreshSnapshot(): Promise<void> {
    return product.refresh().catch(() => {})
  }

  function storageError(error: unknown): void {
    if (!storageErrorReported) {
      storageErrorReported = true
      const title = error instanceof DraftDataError
        ? 'Could not restore the saved draft'
        : 'Drafts remain in this page but could not be saved'
      reportError(title, error, {
        userVisible: true,
      })
    }
  }

  function summaryStatus(
    status: BackendConversation['status'],
  ): Conversation['status'] {
    if (status === 'running' || status === 'compacting') {
      return 'active'
    }
    if (status === 'error') {
      return 'error'
    }
    if (status === 'interrupted' || status === 'stopped') {
      return 'aborted'
    }
    return status === 'completed' ? 'done' : 'idle'
  }

  function metadata(
    record: Pick<
      BackendConversation,
      | 'id'
      | 'title'
      | 'pinned'
      | 'archived'
      | 'target'
      | 'contextVersion'
      | 'revision'
      | 'readRevision'
      | 'unread'
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
      revision: record.revision,
      readRevision: record.readRevision,
      unread: record.unread,
      createdAt: record.createdAt,
      updatedAt: record.updatedAt,
    }
  }

  function newConversation(record: BackendConversation): Conversation {
    return {
      ...metadata(record),
      persistence: 'synced',
      status: summaryStatus(record.status),
      cwd: '/',
      blocks: [],
      phase: 'idle',
      queue: [],
      pendingSteers: [],
      model: {
        providerId: record.providerId ?? '',
        modelId: record.modelId ?? '',
        thinkingEffort: null,
        serviceTierId: null,
      },
      lastError: null,
      draft: '',
      files: [],
      submission: 'idle',
      pendingSend: null,
      messageEdit: null,
      scroll: null,
      attachedHosts: [],
      subagents: [],
      terminals: [],
      load: 'loading',
      pendingAction: null,
    }
  }

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
        const cached = cache.get(current.id)
        if (!cached?.runtime?.connected) {
          current.status = summaryStatus(record.status)
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
  )

  function persisted(conversation: Conversation): SavedDraft {
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
      text: conversation.draft,
      model: { ...conversation.model },
      files: conversation.files.map((file) =>
        isComposerFile(file)
          ? {
              kind: 'file',
              id: file.id,
              name: file.name,
              file: toRaw(file.file),
              upload: file.upload ? { ...file.upload } : null,
            }
          : { ...file },
      ),
      scroll: conversation.scroll
        ? structuredClone(toRaw(conversation.scroll))
        : null,
    }
  }

  function saveDrafts(): void {
    const userId = session.user?.id ?? product.snapshot?.user.id
    if (!userId) {
      return
    }
    const drafts = items.value
      .filter((item) => restored.has(item.id))
      .map((item) => ({
        id: item.id,
        draft: persisted(item),
      }))
    const current = lifetime
    // IndexedDB serializes readwrite transactions; issue them now, including on pagehide.
    for (const entry of drafts) {
      const operation =
        entry.draft.local?.phase === 'draft' &&
        !entry.draft.pendingSend &&
        !entry.draft.text.trim() &&
        !entry.draft.files.length
          ? deleteDraft(userId, entry.id)
          : writeDraft(userId, entry.id, entry.draft)
      void operation.catch((error) => {
        if (current === lifetime) {
          storageError(error)
        }
      })
    }
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
          providerId: draft.model.providerId || null,
          modelId: draft.model.modelId || null,
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
      await product.start()
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
        conversation.draft = draft.text
        conversation.pendingSend = draft.pendingSend
        conversation.messageEdit = restoreMessageEdit(draft.messageEdit ?? null)
        if (conversation.pendingSend && !conversation.pendingSend.error) {
          conversation.pendingSend.error =
            'Sending was interrupted. Retry to confirm delivery.'
        }
        conversation.model = draft.model
        conversation.scroll = draft.scroll
        conversation.files = draft.files.map((file) =>
          file.kind === 'reference'
            ? file
            : {
                ...composerAttachmentFromFile(file.file),
                ...file,
                phase: file.upload ? 'ready' : 'uploading',
              },
        )
      }
    } catch (error) {
      signal.throwIfAborted()
      storageError(error)
      if (error instanceof DraftDataError) {
        throw error
      }
      // An unavailable store permits an in-memory session but cannot authorize overwrites.
      return
    }
    restored.add(conversation.id)
    for (const file of conversation.files) {
      if (isComposerFile(file)) {
        void attachTextSnippet(file, file.file)
      }
      if (isComposerFile(file) && !file.upload) {
        void uploadFile(conversation, file).catch((error) => report('Could not upload the attachment', error))
      }
    }
  }

  async function prepareModel(
    conversation: Conversation,
  ): Promise<ProviderSelection> {
    const pick = composerModel(
      resources.providerInfosFor(conversation.id),
      resources.modelsFor(conversation.id),
      conversation.model.providerId,
      conversation.model.modelId,
    )
    if (pick.kind !== 'ready' || !pick.providerId || !pick.modelId) {
      throw new Error('Choose an available provider and model before sending.')
    }
    const model = product.catalogFor(conversation.id)
      .find((provider) => provider.providerId === pick.providerId)
      ?.models.find((model) => model.id === pick.modelId)
    if (!model) {
      throw new Error('The model catalog changed. Select a model again.')
    }
    conversation.model.providerId = pick.providerId
    conversation.model.modelId = pick.modelId
    const selection = structuredClone(toRaw(model.selection))
    const effort = conversation.model.thinkingEffort
    if (effort !== null) {
      const capability = selection.model.thinking.find(
        (item) => item.type === 'adaptive' || item.type === 'effort',
      )
      if (capability?.type === 'adaptive') {
        selection.thinking = {
          type: 'adaptive',
          effort,
        }
      } else if (capability?.type === 'effort') {
        selection.thinking = {
          type: 'effort',
          effort,
          summary: capability.defaultSummary,
        }
      }
    }
    if (effort === 'disabled') {
      selection.thinking = { type: 'disabled' }
    }
    selection.serviceTierId = conversation.model.serviceTierId
    return {
      providerId: pick.providerId,
      model: selection,
    }
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
      !previous.files.length
    ) {
      saveDrafts()
      items.value = items.value.filter((item) => item !== previous)
      restored.delete(previous.id)
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
      // A different view may have taken over a cached attachment. Navigation
      // is a user action that can reopen it without refetching REST history.
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
      await product.loadModels(conversation.id)
      controller.signal.throwIfAborted()
      await loadHosts(conversation, controller.signal)
      const response = await apiRequest(
        `/conversations/${encodeURIComponent(conversation.id)}/transcript`,
        { signal: controller.signal },
      )
      const transcript = await readResponse(response, transcriptSchema)
      controller.signal.throwIfAborted()
      conversation.blocks = transcript.blocks
      reconcileSubmission(conversation)
      conversation.terminals = transcriptTerminals(transcript.blocks)
      conversation.subagents = transcript.subagents.map((agent) => ({
        ...agent,
        endedAt: agent.endedAt ?? undefined,
      }))
      updateLiveStatus(conversation)
      const pick = composerModel(
        resources.providerInfosFor(conversation.id),
        resources.modelsFor(conversation.id),
        conversation.model.providerId,
        conversation.model.modelId,
      )
      if (conversation.archived || pick.kind !== 'ready') {
        conversation.load = 'ready'
        return
      }
      const runtime = new ConversationRuntime({
        state: conversation,
        prepareModel: () => prepareModel(conversation),
        connect: (signal) => {
          const url = new URL(
            `/api/conversations/${encodeURIComponent(conversation.id)}/stream`,
            window.location.href,
          )
          url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:'
          return connectAgentClient(url.toString(), {
            signal,
            encodeFrame: encodeClientFrame,
            decodeFrame: decodeServerFrame,
          })
        },
        onEvent: (next) => {
          applyConversationEvent(conversation, next)
          reconcileSubmission(conversation)
          if (next.type === 'phase' && next.phase === 'idle') {
            void refreshSnapshot()
          }
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
    const result = await readResponse(response, hostsSchema)
    signal.throwIfAborted()
    conversation.attachedHosts = result.hosts
  }

  async function patch(id: string, changes: unknown): Promise<boolean> {
    const signal = lifetime.signal
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
    const failed = result.results.filter((field) => field.status === 'failed')
    if (failed.length) {
      reportError(
        'Some fields were not updated',
        failed.map((field) => `${field.field}: ${field.message ?? field.code}`).join('\n'),
        { userVisible: true, expected: true },
      )
    }
    await product.revalidate()
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
    changes: Partial<
      Pick<BackendConversation, 'title' | 'pinned' | 'archived' | 'target'>
    >,
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
          const response = await apiRequest('/conversations/batch', {
            method: 'POST',
            signal,
            ...jsonBody({
              items: ids.slice(offset, offset + 100).map((id) => ({
                id,
                patch: changes,
              })),
            }),
          })
          const result = await readResponse(response, conversationBatchSchema)
          signal.throwIfAborted()
          const failures = result.results.flatMap((item) =>
            item.results
              ? item.results
                  .filter((field) => field.status === 'failed')
                  .map(
                    (field) =>
                      `${
                        items.value.find(
                          (conversation) => conversation.id === item.id,
                        )?.title ?? item.id
                      }: ${field.message ?? field.code}`,
                  )
              : [item.message ?? 'Conversation update failed'],
          )
          if (failures.length) {
            success = false
            reportError('Some conversations were not updated', failures.join('\n'), {
              userVisible: true,
              expected: true,
            })
          }
        }
        await product.revalidate()
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
        !item.files.length,
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
      createdAt: now,
      updatedAt: now,
      providerId: null,
      modelId: null,
      status: 'idle',
    })
    conversation.persistence = 'draft'
    conversation.load = 'ready'
    restored.add(conversation.id)
    items.value.unshift(conversation)
    return conversation.id
  }

  async function fork(sourceId: string, request: MessageForkRequest): Promise<string> {
    const signal = lifetime.signal
    const { conversation: record, model } = await forkConversation(sourceId, request, signal)
    await product.refresh()
    signal.throwIfAborted()
    let conversation = items.value.find((item) => item.id === record.id)
    if (!conversation) {
      conversation = newConversation(record)
      items.value.unshift(conversation)
    }
    conversation.model = {
      providerId: model.providerId,
      modelId: model.model.id,
      thinkingEffort: model.thinking?.type === 'disabled'
        ? 'disabled' : model.thinking ? thinkingConfigToEffort(model.thinking) : null,
      serviceTierId: model.serviceTierId ?? null,
    }
    return record.id
  }

  async function persistConversation(
    conversation: Conversation,
  ): Promise<void> {
    if (conversation.persistence === 'synced') {
      return
    }
    const signal = lifetime.signal
    const response = await apiRequest('/conversations', {
      method: 'POST',
      signal,
      ...jsonBody({ id: conversation.id }),
    })
    const result = await readResponse(
      response,
      z.object({ conversation: conversationRecordSchema }),
    )
    signal.throwIfAborted()
    if (
      !(await patch(result.conversation.id, {
        title: conversation.title,
        target: conversation.target,
        pinned: conversation.pinned,
        ...(conversation.model.providerId && conversation.model.modelId
          ? {
              providerId: conversation.model.providerId,
              modelId: conversation.model.modelId,
            }
          : {}),
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
          ...jsonBody({ deviceId: host.deviceId }),
        },
      )
      await apiRequest(
        `/conversations/${encodeURIComponent(
          conversation.id,
        )}/hosts/${encodeURIComponent(host.deviceId)}`,
        {
          method: 'PATCH',
          signal,
          ...jsonBody({ name: host.name }),
        },
      )
    }
    await product.refresh()
    signal.throwIfAborted()
    conversation.persistence = 'synced'
    cache.delete(conversation.id)
    const stored = product.snapshot?.conversations.find(
      (item) => item.id === conversation.id,
    )
    if (stored) {
      Object.assign(conversation, metadata(stored))
    }
  }

  function rename(id: string, title: string): void {
    const conversation = items.value.find((item) => item.id === id)
    if (conversation && conversation.persistence !== 'synced') {
      conversation.title = title
      saveDrafts()
      return
    }
    const signal = lifetime.signal
    void writes
      .run(() => {
        signal.throwIfAborted()
        return patch(id, { title })
      })
      .catch((error) => report('Could not rename the conversation', error))
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
        }),
      })
      await product.revalidate()
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
        ...jsonBody({ revision }),
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
          ...jsonBody({ deviceId }),
        },
      )
      await loadHosts(conversation)
    } catch (error) {
      report('Could not attach the device', error)
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
          ...jsonBody({ name }),
        },
      )
      await loadHosts(conversation)
    } catch (error) {
      report('Could not rename the project', error)
    }
  }

  async function selectModel(
    conversation: Conversation,
    providerId: string,
    modelId: string,
  ): Promise<void> {
    if (conversation.persistence !== 'synced') {
      conversation.model = {
        providerId,
        modelId,
        thinkingEffort: null,
        serviceTierId: null,
      }
      saveDrafts()
      return
    }
    const signal = lifetime.signal
    try {
      await writes.run(async () => {
        signal.throwIfAborted()
        if (
          !(await patch(conversation.id, {
            providerId,
            modelId,
          }))
        ) {
          return
        }
        signal.throwIfAborted()
        conversation.model = {
          providerId,
          modelId,
          thinkingEffort: null,
          serviceTierId: null,
        }
        const runtime = cache.get(conversation.id)?.runtime
        if (runtime) {
          await runtime.setModel()
        } else {
          await reloadSession(conversation.id)
        }
      })
    } catch (error) {
      report('Could not change the model', error)
    }
  }

  function setThinking(
    conversation: Conversation,
    thinking: ThinkingConfig | null,
  ): void {
    conversation.model.thinkingEffort =
      thinking?.type === 'adaptive' || thinking?.type === 'effort'
        ? thinking.effort
        : thinking?.type === 'disabled'
          ? 'disabled'
          : null
    void cache.get(conversation.id)?.runtime?.setModel().catch((error) => report('Could not change the model', error))
  }

  function setTier(conversation: Conversation, tier: string | null): void {
    conversation.model.serviceTierId = tier
    void cache.get(conversation.id)?.runtime?.setModel().catch((error) => report('Could not change the model', error))
  }

  async function send(conversation: Conversation): Promise<void> {
    if (
      conversation.archived ||
      conversation.submission === 'sending' ||
      (!conversation.pendingSend && !attachmentsReady(conversation.files)) ||
      (!conversation.pendingSend &&
        !conversation.draft.trim() &&
        !conversation.files.length)
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
        fileIds: conversation.files.map((file) => file.id),
        error: null,
      }
      conversation.draft = ''
    }
    const pending = conversation.pendingSend
    const signal = lifetime.signal
    pending.error = null
    const draft = pending.text
    const files = conversation.files.filter((file) =>
      pending.fileIds.includes(file.id),
    )
    conversation.submission = 'sending'
    saveDrafts()
    try {
      await persistConversation(conversation)
      const content: UserContentBlock[] = draft.trim()
        ? [
            {
              type: 'text',
              text: draft.trim(),
            },
          ]
        : []
      for (const file of files) {
        if (!isComposerFile(file)) {
          content.push(
            contentReference({
              type: 'remote_file',
              deviceId: file.deviceId,
              path: file.path,
            }),
          )
        } else if (file.upload) {
          content.push(
            contentReference({
              type: 'upload',
              ref: file.upload.id,
              fileName: file.name,
            }),
          )
        } else {
          throw new Error(`${file.name} has not finished uploading.`)
        }
      }
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

  function clearSubmission(conversation: Conversation, id: string): void {
    const pending = conversation.pendingSend
    if (pending?.id !== id) {
      return
    }
    conversation.pendingSend = null
    for (const fileId of pending.fileIds) {
      removeFile(conversation, fileId)
    }
    saveDrafts()
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
    lifetime.abort()
    lifetime = new AbortController()
    uploads.dispose(items.value)
    items.value = []
    restored.clear()
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
    markRead,
    reloadList: refreshSnapshot,
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
    switchTarget: (id: string, target: BackendConversation['target']) =>
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
    renameHost: (conversation: Conversation, deviceId: string, name: string) =>
      changeConversations(
        [conversation.id],
        () => renameHost(conversation, deviceId, name),
        undefined,
      ),
    selectModel,
    setThinking,
    setTier,
    send,
    editVersion: (conversation: Conversation) =>
      cache.get(conversation.id)?.runtime?.transcriptVersion() ?? null,
    submitEdit: (conversation: Conversation) => submitMessageEdit({
      get: () => conversation.messageEdit,
      set: (state) => { conversation.messageEdit = state },
      send: async (request) => {
        const signal = lifetime.signal
        const runtime = await runtimeFor(conversation)
        const content = await loadEditContent(request.content, signal)
        signal.throwIfAborted()
        await runtime.editAndSend({ ...request, content })
      },
    }),
    addFiles,
    removeFile,
    saveDrafts,
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
      action(conversation, (runtime) =>
        conversation.phase === 'idle'
          ? runtime.sendQueuedMessage(id)
          : runtime.steerQueuedMessage(id),
      ),
    removePendingSteer: (conversation: Conversation, id: string) =>
      action(conversation, (runtime) => runtime.deletePendingSteer(id)),
    interruptWithSteer: (conversation: Conversation, id: string) =>
      action(conversation, (runtime) => runtime.interruptPendingSteer(id)),
  }
})
