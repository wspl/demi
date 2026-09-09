import { computed, ref, toRaw, watch } from 'vue'
import { defineStore } from 'pinia'
import { z } from 'zod'
import { SerialQueue } from '@demicodes/utils'
import {
  type ThinkingConfig,
  type UserContentBlock,
} from '@demicodes/core'
import { ConversationRuntime } from '@demicodes/web-ui/agent/conversation-runtime'
import { composerModel } from '@demicodes/web-ui/agent/model-selection'
import {
  attachmentsReady,
  composerAttachmentFromFile,
  isComposerFile,
} from '@demicodes/web-ui/agent/message-input/attachments'
import { connectAgentClient } from '@demicodes/web-ui/transport/agent-socket'
import {
  type ProviderSelection,
} from '@demicodes/web-ui/transport/protocol'
import { apiRequest, jsonBody, readResponse } from '../api/client'
import {
  conversationBatchSchema,
  conversationRecordSchema,
  conversationUpdateSchema,
  hostsSchema,
  type BackendConversation,
} from '../api/contracts'
import {
  contentReference,
  decodeServerFrame,
  encodeClientFrame,
  transcriptSchema,
} from '../api/transcript'
import { createConversationUploads } from './uploads'
import { useResources } from '../state/resources'
import { useProduct } from '../state/product'
import { useSession } from '../auth/session'
import type { Conversation } from '../state/types'
import { applyConversationEvent, updateLiveStatus } from './activity'
import { transcriptTerminals } from './terminals'
import { readDraft, writeDraft, type SavedDraft } from './drafts'

export const useConversations = defineStore('conversations', () => {
  const product = useProduct()
  const resources = useResources()
  const session = useSession()
  const items = ref<Conversation[]>([])
  const notice = ref('')
  const listStatus = computed(() => product.load)
  const writes = new SerialQueue()
  const restored = new Set<string>()
  const uploads = createConversationUploads(saveDrafts, report)
  const { uploadFile, addFiles, retryFile, removeFile } = uploads
  let lifetime = new AbortController()
  let activeController: AbortController | null = null
  let activeRuntime: {
    id: string
    runtime: ConversationRuntime
  } | null = null
  let draftTimer: ReturnType<typeof setTimeout> | null = null
  let storageErrorReported = false

  function report(error: unknown): void {
    if (error instanceof DOMException && error.name === 'AbortError') {
      return
    }
    if (!lifetime.signal.aborted) {
      notice.value = error instanceof Error ? error.message : String(error)
    }
  }

  function storageError(error: unknown): void {
    if (!storageErrorReported) {
      storageErrorReported = true
      notice.value = `Drafts remain in this page but could not be saved: ${error instanceof Error ? error.message : String(error)}`
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

  function metadata(record: BackendConversation) {
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
      scroll: null,
      attachedHosts: [],
      subagents: [],
      terminals: [],
      load: 'loading',
    }
  }

  watch(
    () => product.snapshot,
    (snapshot) => {
      if (!snapshot) {
        return
      }
      items.value = snapshot.conversations.map((record) => {
        const current = items.value.find((item) => item.id === record.id)
        if (!current) {
          return newConversation(record)
        }
        const revisionChanged = current.revision !== record.revision
        const contextChanged = current.contextVersion !== record.contextVersion
        const archiveChanged = current.archived !== record.archived
        Object.assign(current, metadata(record))
        if (activeRuntime?.id !== current.id || !activeRuntime.runtime.connected) {
          current.status = summaryStatus(record.status)
        }
        if (contextChanged) {
          for (const file of current.files) {
            if (isComposerFile(file) && file.destination === 'workspace') {
              file.upload = null
              void uploadFile(current, file).catch(report)
            }
          }
        }
        if (
          revisionChanged &&
          activeController &&
          product.activeConversationId === current.id
        ) {
          void loadHosts(current, activeController.signal).catch(report)
        }
        if (
          (contextChanged || archiveChanged) &&
          product.activeConversationId === current.id
        ) {
          void activate(current.id)
        }
        return current
      })
      if (product.activeConversationId && !activeController) {
        void activate(product.activeConversationId)
      }
    },
  )

  function persisted(conversation: Conversation): SavedDraft {
    return {
      text: conversation.draft,
      model: { ...conversation.model },
      files: conversation.files.map((file) =>
        isComposerFile(file)
          ? {
              kind: 'file',
              id: file.id,
              name: file.name,
              destination: file.destination,
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
    if (draftTimer !== null) {
      clearTimeout(draftTimer)
    }
    draftTimer = null
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
      void writeDraft(userId, entry.id, entry.draft).catch(error => {
        if (current === lifetime) {
          storageError(error)
        }
      })
    }
  }

  watch(
    () =>
      items.value.map((item) => ({
        id: item.id,
        draft: item.draft,
        files: item.files,
        model: item.model,
        scroll: item.scroll,
      })),
    () => {
      if (draftTimer !== null) {
        clearTimeout(draftTimer)
      }
      draftTimer = null
      if (items.value.length && session.signedIn) {
        draftTimer = setTimeout(saveDrafts, 250)
      }
    },
    { deep: true },
  )

  async function restoreDraft(
    conversation: Conversation,
    signal: AbortSignal,
  ): Promise<void> {
    if (restored.has(conversation.id) || !session.user) {
      return
    }
    try {
      const draft = await readDraft(session.user.id, conversation.id)
      signal.throwIfAborted()
      if (draft) {
        conversation.draft = draft.text
        conversation.model = draft.model
        conversation.scroll = draft.scroll
        conversation.files = draft.files.map((file) =>
          file.kind === 'reference'
            ? file
            : {
                ...composerAttachmentFromFile(file.file, null),
                ...file,
                phase: file.upload ? 'ready' : 'uploading',
              },
        )
      }
    } catch (error) {
      signal.throwIfAborted()
      storageError(error)
    }
    restored.add(conversation.id)
    for (const file of conversation.files) {
      if (
        isComposerFile(file) &&
        (!file.upload ||
          (file.upload.kind === 'workspace' &&
            file.upload.contextVersion !== conversation.contextVersion))
      ) {
        void uploadFile(conversation, file).catch(report)
      }
    }
  }

  function selectedCatalog(conversation: Conversation) {
    return product.catalogs[conversation.id] ?? product.catalogs[''] ?? []
  }

  async function prepareModel(
    conversation: Conversation,
  ): Promise<ProviderSelection> {
    const pick = composerModel(
      resources.providerInfos,
      resources.models,
      conversation.model.providerId,
      conversation.model.modelId,
    )
    if (pick.kind !== 'ready' || !pick.providerId || !pick.modelId) {
      throw new Error('Choose an available provider and model before sending.')
    }
    const model = selectedCatalog(conversation)
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

  function releaseActive(): void {
    activeController?.abort()
    activeController = null
    activeRuntime?.runtime.dispose()
    activeRuntime = null
  }

  async function activate(id: string | null): Promise<void> {
    releaseActive()
    product.activeConversationId = id
    const conversation = items.value.find((item) => item.id === id)
    if (!conversation) {
      return
    }
    const controller = new AbortController()
    activeController = controller
    conversation.load = conversation.blocks.length ? 'reconnecting' : 'loading'
    try {
      await restoreDraft(conversation, controller.signal)
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
      conversation.terminals = transcriptTerminals(transcript.blocks)
      conversation.subagents = transcript.subagents.map((agent) => ({
        ...agent,
        endedAt: agent.endedAt ?? undefined,
      }))
      conversation.load = 'ready'
      updateLiveStatus(conversation)
      const pick = composerModel(
        resources.providerInfos,
        resources.models,
        conversation.model.providerId,
        conversation.model.modelId,
      )
      if (conversation.archived || pick.kind !== 'ready') {
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
        onEvent: next => {
          applyConversationEvent(conversation, next)
          if (next.type === 'phase' && next.phase === 'idle') {
            void product.refresh().catch(report)
          }
        },
      })
      activeRuntime = {
        id: conversation.id,
        runtime,
      }
      await runtime.connect()
    } catch (error) {
      if (!controller.signal.aborted) {
        conversation.load = 'failed'
        conversation.lastError =
          error instanceof Error ? error.message : String(error)
      }
    }
  }

  async function runtimeFor(
    conversation: Conversation,
  ): Promise<ConversationRuntime> {
    if (conversation.archived) {
      throw new Error('Restore this conversation before sending.')
    }
    if (activeRuntime?.id !== conversation.id) {
      await activate(conversation.id)
    }
    if (activeRuntime?.id !== conversation.id) {
      throw new Error(
        conversation.lastError ?? 'No available model for this conversation.',
      )
    }
    return activeRuntime.runtime
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
    const response = await apiRequest(`/conversations/${encodeURIComponent(id)}`, {
      method: 'PATCH',
      signal: lifetime.signal,
      ...jsonBody(changes),
    })
    const result = await readResponse(response, conversationUpdateSchema)
    signal.throwIfAborted()
    const failed = result.results.filter((field) => field.status === 'failed')
    if (failed.length) {
      notice.value = failed
        .map((field) => `${field.field}: ${field.message ?? field.code}`)
        .join('\n')
    }
    await product.revalidate()
    return failed.length === 0
  }

  async function batch(ids: string[], changes: unknown): Promise<boolean> {
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
                      `${items.value.find((conversation) => conversation.id === item.id)?.title ?? item.id}: ${field.message ?? field.code}`,
                  )
              : [item.message ?? 'Conversation update failed'],
          )
          if (failures.length) {
            success = false
            notice.value = failures.join('\n')
          }
        }
        await product.revalidate()
        return success
      })
    } catch (error) {
      report(error)
      return false
    }
  }

  async function create(projectId: string | null = null): Promise<string | null> {
    const signal = lifetime.signal
    try {
      const response = await apiRequest('/conversations', {
        method: 'POST',
        signal: lifetime.signal,
      })
      const result = await readResponse(
        response,
        z.object({ conversation: conversationRecordSchema }),
      )
      signal.throwIfAborted()
      if (projectId) {
        await patch(result.conversation.id, {
          target: {
            kind: 'workspace',
            workspaceId: projectId,
          },
        })
      } else {
        await product.revalidate()
      }
      return result.conversation.id
    } catch (error) {
      report(error)
      return null
    }
  }

  function rename(id: string, title: string): void {
    const signal = lifetime.signal
    void writes
      .run(() => {
        signal.throwIfAborted()
        return patch(id, { title })
      })
      .catch(report)
  }

  async function reorder(id: string, beforeId: string | null): Promise<void> {
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
      report(error)
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
      report(error)
    }
  }

  async function attachHost(
    conversation: Conversation,
    deviceId: string,
  ): Promise<void> {
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
      report(error)
    }
  }

  async function detachHost(
    conversation: Conversation,
    deviceId: string,
  ): Promise<void> {
    try {
      await apiRequest(
        `/conversations/${encodeURIComponent(conversation.id)}/hosts/${encodeURIComponent(deviceId)}`,
        {
          method: 'DELETE',
          signal: lifetime.signal,
        },
      )
      await loadHosts(conversation)
    } catch (error) {
      report(error)
    }
  }

  async function renameHost(
    conversation: Conversation,
    deviceId: string,
    name: string,
  ): Promise<void> {
    try {
      await apiRequest(
        `/conversations/${encodeURIComponent(conversation.id)}/hosts/${encodeURIComponent(deviceId)}`,
        {
          method: 'PATCH',
          signal: lifetime.signal,
          ...jsonBody({ name }),
        },
      )
      await loadHosts(conversation)
    } catch (error) {
      report(error)
    }
  }

  async function selectModel(
    conversation: Conversation,
    providerId: string,
    modelId: string,
  ): Promise<void> {
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
        if (activeRuntime?.id === conversation.id) {
          await activeRuntime.runtime.setModel()
        } else {
          await activate(conversation.id)
        }
      })
    } catch (error) {
      report(error)
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
    if (activeRuntime?.id === conversation.id) {
      void activeRuntime.runtime.setModel().catch(report)
    }
  }

  function setTier(conversation: Conversation, tier: string | null): void {
    conversation.model.serviceTierId = tier
    if (activeRuntime?.id === conversation.id) {
      void activeRuntime.runtime.setModel().catch(report)
    }
  }

  async function send(conversation: Conversation): Promise<void> {
    if (
      conversation.submission === 'sending' ||
      !attachmentsReady(conversation.files) ||
      (!conversation.draft.trim() && !conversation.files.length)
    ) {
      return
    }
    const draft = conversation.draft
    const files = [...conversation.files]
    conversation.submission = 'sending'
    try {
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
        } else if (file.upload?.kind === 'message') {
          content.push(
            contentReference({
              type: file.upload.media,
              source: {
                type: 'ref',
                ref: file.upload.id,
                fileName: file.name,
              },
            }),
          )
        } else if (
          file.upload?.kind === 'workspace' &&
          file.upload.contextVersion === conversation.contextVersion
        ) {
          content.push({
            type: 'reference',
            reference: file.upload.path,
          })
        } else {
          throw new Error(
            `${file.name} has not finished uploading to this environment.`,
          )
        }
      }
      await (await runtimeFor(conversation)).submit(content)
      if (conversation.draft === draft) {
        conversation.draft = ''
      }
      for (const file of files) {
        removeFile(conversation, file.id)
      }
      saveDrafts()
    } catch (error) {
      report(error)
    } finally {
      conversation.submission = 'idle'
    }
  }

  function action(
    conversation: Conversation,
    operation: (runtime: ConversationRuntime) => Promise<void>,
  ): void {
    void runtimeFor(conversation).then(operation).catch(report)
  }

  function stopAll(): void {
    saveDrafts()
    releaseActive()
    lifetime.abort()
    lifetime = new AbortController()
    uploads.dispose(items.value)
    items.value = []
    restored.clear()
    notice.value = ''
    storageErrorReported = false
  }

  return {
    items,
    notice,
    listStatus,
    activate,
    create,
    reorder,
    rename,
    markRead,
    reloadList: () => product.refresh().catch(report),
    reloadSession: (id: string) => activate(id),
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
    attachHost,
    detachHost,
    renameHost,
    selectModel,
    setThinking,
    setTier,
    send,
    addFiles,
    removeFile,
    retryFile,
    saveDrafts,
    stopAll,
    abortSubagents: (conversation: Conversation) =>
      action(conversation, (runtime) => runtime.abortSubagents()),
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
