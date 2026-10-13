import type { MessageAgents } from '../agent/agent-links'
import type { FileContents } from '../files/types'
import type { HostImage } from '../files/host-image'

/**
 * An attachment the agent uploaded to the conversation (`commands.md`
 * § Attachment commands), as a message shows it.
 */
export interface MessageAttachment {
  /** The file's name. */
  name: string
  /** The media type the backend read from its bytes. */
  mediaType: string
  /** Where its bytes load from, shown as its media type. */
  url: string
  /** Its bytes' SHA-256 in the user's blobs, which a drag of it into a composer names it by. */
  sha256: string
  /**
   * An image's or a video's size in pixels, which its record carries when
   * the backend read it from its header (`web-api.md`).
   */
  width?: number
  height?: number
}

/**
 * What the page knows of an attachment a message names: still asking, the
 * attachment, no attachment of the conversation by that number, or that
 * asking failed.
 */
export type AttachmentLookup =
  | { state: 'loading' }
  | { state: 'found'; attachment: MessageAttachment }
  | { state: 'missing' }
  | { state: 'failed' }

/**
 * A Host file a message names, by its absolute path, and the line it opens
 * at, if any; an empty path names no file, only a directory's tree. `root`
 * is the directory the view opens for, the working directory of the agent
 * that wrote the link; absent, the conversation's.
 */
export interface MessageFileTarget {
  path: string
  line: number | null
  root?: string
}

/**
 * What the host offers for the files a conversation's messages name on its
 * Host and the attachments they name (`file-previews.md` § Files named in
 * messages).
 */
export interface ConversationFiles {
  /**
   * The Host's home, which a path starting `~/` starts from; absent or null
   * while it is not known, and such a path then names no file.
   */
  home?: string | null
  /**
   * The Host's files a message's images load from: each image shows the
   * version of its file the page keeps, which the Host's watch keeps current
   * (`plugin-pages.md` § What the service keeps), so it changes when the file
   * does. Absent while the Host is not known, and a Host image then shows its
   * alt text.
   */
  contents?: FileContents
  /**
   * Shows the file at a Host path, at its line when it names one, through
   * the `file` intent; absent while no plugin the user has on opens it, so a
   * link to a file shows as text and a Host image only shows
   * (`file-previews.md` § Files named in messages). What the panel already
   * shows stays as it is (`openIntent`).
   */
  open?(target: MessageFileTarget): void
  /**
   * Which of `paths`, the Host paths a reply's inline code names, are files
   * there, as the Host answered one question about them all: null until it
   * answered, or while it cannot, such as while the Cloud is stopped. A
   * render reads it as it is now and renders again when the answer arrives;
   * absent, inline code stays code.
   */
  codeFiles?(paths: readonly string[]): ReadonlySet<string> | null
  /**
   * The conversation's attachment `id`, such as `a3`. A render reads it as it
   * is now and renders again when the answer arrives; absent, an attachment
   * shows its alt text.
   */
  attachment?(id: string): AttachmentLookup
}

/** The same for one message, with the working directory its relative paths resolve against. */
export interface MessageFiles extends ConversationFiles {
  cwd: string
}

export interface MarkdownRenderOptions {
  /** Absent, a message's paths stay text and its Host images show their alt text. */
  files?: MessageFiles
  /** The paths of the message's inline code the Host has as files, which show as links to them. */
  codeFiles?: ReadonlySet<string> | null
  /**
   * What shows of each Host image the message names, by its absolute path
   * (`useHostImagesAt`); one it does not name yet is still being read, and
   * shows its alt text meanwhile.
   */
  hostImages?: ReadonlyMap<string, HostImage>
  /** The conversation's agents, which `agent:4` links; absent, such a link shows its text alone. */
  agents?: MessageAgents
}
