import { reactive } from 'vue'
import type { UserContentBlock } from '@demicodes/protocol'
import {
  AttachmentUploadQueue,
  applyAttachmentUpdate,
  attachingUpdate,
  attachmentFileError,
  composerAttachment,
  composerAttachmentFromFile,
  composerRemoteAttachment,
  encodeRemoteReference,
  filePreviewUrl,
  isComposerFile,
  type ClaimFile,
  type ComposerAttachment,
  type UploadFile,
  type UploadedFile,
} from '../message-input/attachments'
import { joinMessageContent } from '../message-input/message-content'
import { composerCapsule, composerTransfer } from '../message-editor/capsules'
import type { DraftMessage, MessageFiles } from './comments'

/**
 * The files of the messages a page writes beside a composer it does not
 * share them with, as a gallery specimen's or a demo's comments and side
 * questions: each taken, uploaded through `upload` and pasted as the
 * product's conversation takes its files. The page keeps every file it took
 * while it is open, so `hold` keeps nothing.
 */
export interface PageMessageFiles {
  files: MessageFiles
  /**
   * `message` as a sent message holds it, each file where its mark stands,
   * once its upload is done.
   */
  content: (message: DraftMessage) => Promise<UserContentBlock[]>
}

/**
 * The page's stand-in for the backend's files: an upload of a file's bytes,
 * the claim of a stored file a pasted message names, and where a stored
 * file's picture loads from.
 */
export interface PageFileHost {
  upload: UploadFile
  claim: ClaimFile
  picture: (sha256: string, mediaType: string) => string | undefined
}

/** Files taken and pasted through the page's `host`. */
export function pageMessageFiles(host: PageFileHost): PageMessageFiles {
  const { upload, claim } = host
  const held = reactive<ComposerAttachment[]>([])
  const uploads = new AttachmentUploadQueue()
  const bytes = new Map<string, File>()
  const answers = new Map<string, UploadedFile>()
  const running = new Map<string, Promise<void>>()
  let nextAttachment = 1

  const find = (id: string) => held.find((item) => item.id === id)

  function start(id: string, work: (signal: AbortSignal, report: (fraction: number) => void) => Promise<UploadedFile>): Promise<void> {
    const item = find(id)
    if (!item || !isComposerFile(item))
      return Promise.resolve()
    const transfer = uploads.start(id, async (signal, report) => {
      const answer = await work(signal, report)
      answers.set(id, answer)
      item.snippet = answer.snippet
    }, (update) => applyAttachmentUpdate(item, attachingUpdate(update, bytes.has(id))))
      .then(() => {}, () => applyAttachmentUpdate(item, { phase: 'failed' }))
    running.set(id, transfer)
    return transfer
  }

  const files: MessageFiles = {
    add(picked) {
      return picked.flatMap((file) => {
        if (attachmentFileError(file))
          return []
        const item = composerAttachmentFromFile(file)
        held.push(item)
        bytes.set(item.id, file)
        void start(item.id, (signal, progress) => upload(file, { signal, progress }))
        return [composerCapsule(find(item.id)!)]
      })
    },
    capsules: (ids) => ids.flatMap((id) => {
      const item = find(id)
      return item ? [composerCapsule(item)] : []
    }),
    transfers: {
      transfer: (id) => {
        const item = find(id)
        return item ? composerTransfer(item) : undefined
      },
      carries: (id) => find(id) !== undefined,
      retry: (id) => {
        const file = bytes.get(id)
        if (file)
          void start(id, (signal, progress) => upload(file, { signal, progress }))
      },
      current: (id) => {
        const item = find(id)
        return item ? composerCapsule(item) : undefined
      },
    },
    paste: {
      claim: (file, picture) => {
        const item = composerAttachment({ name: file.name, src: host.picture(file.sha256, file.mediaType), phase: 'attaching' })
        held.push(item)
        const kept = start(item.id, async (signal, progress) => {
          const answer = await claim(file, signal)
          if (answer)
            return answer
          if (!picture)
            throw new Error(`${file.name} is not among this account's files.`)
          bytes.set(item.id, picture)
          const shown = find(item.id)
          if (shown && isComposerFile(shown))
            shown.src = filePreviewUrl(picture) ?? shown.src
          return upload(picture, { signal, progress })
        }).then(() => answers.has(item.id))
        return { capsule: composerCapsule(find(item.id)!), kept }
      },
      remote: (file) => {
        const item = composerRemoteAttachment({ host: file.deviceId, path: file.path })
        held.push(item)
        return composerCapsule(item)
      },
    },
    hold: () => {},
  }

  /** A file as a sent message records it: its picture first, then its record, or a device's file. */
  async function sent(id: string): Promise<UserContentBlock[]> {
    const item = find(id)
    if (!item)
      throw new Error('A file of this message is no longer available. Attach it again.')
    if (item.kind === 'reference')
      return [{ type: 'reference', reference: encodeRemoteReference(item.host, item.path, item.host) }]
    await running.get(id)
    const answer = answers.get(id)
    if (!answer)
      throw new Error(`${item.name} did not upload.`)
    const record: UserContentBlock = {
      type: 'attachment',
      name: item.name,
      id: `a${nextAttachment++}`,
      mediaType: answer.mediaType,
      sizeBytes: bytes.get(id)?.size ?? 0,
      sha256: answer.sha256,
      ...(item.snippet ? { snippet: item.snippet } : {}),
    }
    return item.src ? [{ type: 'image', source: { type: 'url', url: item.src } }, record] : [record]
  }

  return {
    files,
    content: async (message) => joinMessageContent(message.text, await Promise.all(message.fileIds.map(sent))),
  }
}
