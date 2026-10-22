import type { MessageAgents } from '../agent/agent-links'
import type { FileContents } from '../files/types'
import type { PictureCopy } from '@demicodes/protocol'

/**
 * An attachment of the conversation, sent in a message or uploaded by the agent
 * (`crates/backend-host-access/src/attachment_commands.rs`), as a message shows
 * it. /
 */
export interface MessageAttachment {
  name: string
  /** The media type the backend read from its bytes. */
  mediaType: string
  /** Where its bytes load from, shown as its media type. */
  url: string
  /** Its size in bytes. */
  size: number
  /**
   * Its bytes' SHA-256 in the user's blobs, which a drag of it into a composer
   * names it by.
   */
  sha256: string
  /**
   * An image's or a video's size in pixels, which its record carries when
   * the backend read it from its header.
   */
  width?: number
  height?: number
  /**
   * The file it was uploaded from, when its upload recorded it: the id of
   * the Host the upload ran on and the file's full path there. Absent for a
   * file the user sent and for an attachment uploaded before uploads
   * recorded it.
   */
  source?: AttachmentOrigin
}

/** The file of a Host an attachment was uploaded from. */
export interface AttachmentOrigin {
  host: string
  path: string
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
 * Host and the attachments they name.
 */
export interface ConversationFiles {
  /**
   * The Host's home, which a path starting `~/` starts from; absent or null
   * while it is not known, and such a path then names no file.
   */
  home?: string | null
  /**
   * The Host's files, read through the page's kept versions of them, which
   * the Host's watch keeps current: a file's download, and the files a
   * message the user wrote names. Absent while the Host is not known.
   */
  contents?: FileContents
  /**
   * The device the conversation's Host is, which tells a file the user
   * attached from it, read through `contents`, from one on another device;
   * absent while it is not known.
   */
  device?: string
  /**
   * Shows the file at a Host path, at its line when it names one, through
   * the `file` intent; absent while no plugin the user has on opens it, so a
   * link to a file shows as text and a Host image only shows. What the panel
   * already shows stays as it is (`openIntent`).
   */
  open?(target: MessageFileTarget): void
  /**
   * Shows the file at a Host path in the File view's tree, selected and in
   * view, through the `file` intent; absent while no plugin the user has on
   * opens it, and a file link's menu then has no Show in Files.
   */
  showInFiles?(target: Pick<MessageFileTarget, 'path' | 'root'>): void
  /**
   * The conversation's attachment `id`, such as `a3`: a reply shows any
   * attachment of its conversation, whoever uploaded it. A render reads it as
   * it is now and renders again when the answer arrives; absent, an
   * attachment shows its alt text.
   */
  attachment?(id: string): AttachmentLookup
  /**
   * Opens attachment `id`, a file that is no image, video or audio, in the
   * work panel through the `attachment` intent, previewed from its copy; absent
   * while no plugin the user has on opens it, as on a share, and its card then
   * only offers Download.
   */
  openAttachment?(id: string, attachment: MessageAttachment): void
  /**
   * The full path on the conversation's Host of the file `attachment` was
   * uploaded from, which a card of a file that is no image or video opens in
   * the Files panel, as a link to it does: its Host is the conversation's and
   * online, and the file is there, which the page asks the Host once for each
   * opening. Null otherwise, and at once, with no request and no wake, while
   * the Host is offline or its Cloud stopped; the card then opens the copy.
   * Absent, every card opens its copy.
   */
  attachmentOrigin?(id: string, attachment: MessageAttachment): Promise<string | null>
  /**
   * Shows a commit a message links as `commit:3f2a9c1`, or as
   * `commit:src/v8@86229da` in the repository of that folder, in the Change
   * view's Commits mode, through the `commit` intent; absent while no plugin
   * the user has on opens it, so such a link shows as text. /
   */
  openCommit?(target: MessageCommitTarget): void
  /**
   * The full hash of the commit a message links by an abbreviation, as the
   * repository the link's directory is in reads it; rejects when it has no
   * such commit or cannot be read. Absent, a commit link's menu has no Copy
   * Commit Hash.
   */
  commitHash?(target: MessageCommitTarget): Promise<string>
}

/**
 * A commit a message links, by its id or an abbreviation; `root`, the
 * folder whose repository has it, the one the link names or else the
 * directory of the agent that wrote it, absent for the conversation's.
 */
export interface MessageCommitTarget {
  commit: string
  root?: string
}

/**
 * The same for one message, with the working directory its relative paths
 * resolve against.
 */
export interface MessageFiles extends ConversationFiles {
  cwd: string
}

/**
 * What a reply keeps of the pictures of the Host it shows
 * (`docs/decisions/0046-a-reply-keeps-the-pictures-it-shows.md`): the copy
 * the backend made of each, by the target as the reply wrote it.
 */
export interface MessagePictures {
  copies: ReadonlyMap<string, PictureCopy>
  /**
   * Whether the reply is still arriving: the backend copies its pictures
   * once it is complete, so a picture it names has no copy yet and shows as a
   * placeholder.
   */
  streaming: boolean
}

export interface MarkdownRenderOptions {
  /**
   * Absent, a message's paths stay text and its Host images show their alt
   * text.
   */
  files?: MessageFiles
  /**
   * The Host files the message's inline code names, by the code that names
   * each, as the backend kept them with the reply, which show as links to
   * them.
   */
  codeFiles?: ReadonlyMap<string, MessageFileTarget> | null
  /**
   * What the reply keeps of the pictures of the Host it shows; absent, such a
   * picture shows as a link to its path.
   */
  pictures?: MessagePictures
  /**
   * The conversation's agents, which `agent:4` links; absent, such a link shows
   * its text alone.
   */
  agents?: MessageAgents
}
