import { TEXT_FILE_BYTES } from '@demicodes/protocol'
import { HostFiles } from './file-cache'
import { keptSource } from './kept-source'
import { isBinary } from './text-start'
import { FileBrowserError, type FileBrowserSource, type FileDescription } from './types'

/** An attachment's file as its preview reads it: its name, its size in bytes, and where its stored copy loads from. */
export interface AttachmentFile {
  name: string
  size: number
  url: string
}

/** Where an attachment's file stands in its source: at the root, by its name. */
export function attachmentPath(file: Pick<AttachmentFile, 'name'>): string {
  return `/${file.name}`
}

/**
 * The source an attachment's preview reads, so the previews of the File view show
 * the attachment's stored copy: one file at the root, which never changes, so
 * nothing watches it.
 *
 * - Text is read whole up to the text route's limit and refused as binary by its
 *   rule. A larger file's start is read with a byte range.
 * - Its bytes and Download come from the copy's address.
 * - Nothing else is there.
 */
export function attachmentSource(file: AttachmentFile): FileBrowserSource {
  const path = attachmentPath(file)
  const here = (at: string) => {
    if (at !== path)
      throw new FileBrowserError('not-found')
  }
  const read = async (range: string | null, signal?: AbortSignal): Promise<Uint8Array> => {
    const response = await fetch(file.url, { ...(range === null ? {} : { headers: { Range: range } }), ...(signal ? { signal } : {}) })
    if (!response.ok)
      throw new FileBrowserError(response.status === 404 ? 'not-found' : 'other')
    return new Uint8Array(await response.arrayBuffer())
  }
  const description: FileDescription = { size: file.size, modifiedAt: null, version: null }
  return keptSource({
    platform: 'linux',
    home: '/',
    list: async (at) => {
      if (at !== '/')
        throw new FileBrowserError('not-found')
      return { value: [{ name: file.name, isDirectory: false, size: file.size }], readAt: null }
    },
    readText: async (at) => {
      here(at)
      if (file.size > TEXT_FILE_BYTES)
        throw new FileBrowserError('too-large')
      const bytes = await read(null)
      if (isBinary(bytes))
        throw new FileBrowserError('binary')
      return { value: { text: new TextDecoder().decode(bytes), version: null }, readAt: null }
    },
    contents: {
      url: () => file.url,
      describe: async (at) => {
        here(at)
        return { value: description, readAt: null }
      },
      readStart: async (at, length, options) => {
        here(at)
        return read(`bytes=0-${length - 1}`, options?.signal)
      },
    },
  }, { files: new HostFiles() })
}
