import type { MessageAgents } from '../agent/agent-links'
import type { FileContents } from '../files/types'
import type { HostImage } from '../files/host-image'

/**
 * An attachment of the conversation, sent in a message or uploaded by the agent (`crates/backend-host-access/src/attachment_commands.rs`), as a message shows it.
 */
export interface MessageAttachment {
  /** The file's name. */
  name: string
  /** The media type the backend read from its bytes. */
  mediaType: string
  /** Where its bytes load from, shown as its media type. */
  url: string
  /** Its size in bytes. */
  size: number
  /** Its bytes' SHA-256 in the user's blobs, which a drag of it into a composer names it by. */
  sha256: string
  /**
   * An image's or a video's size in pixels, which its record carries when
   * the backend read it from its header.
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
 * Host and the attachments they name.
 */
export interface ConversationFiles {
  /**
   * The Host's home, which a path starting `~/` starts from; absent or null
   * while it is not known, and such a path then names no file.
   */
  home?: string | null
  /**
   * The Host's files a message's images load from: each image shows the version of its file the
   * page keeps, which the Host's watch keeps current, so it changes when the file does. Absent
   * while the Host is not known, and a Host image then shows its alt text.
   */
  contents?: FileContents
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
   * Which of `paths`, the Host paths a reply's inline code names, are files
   * there, as the Host answered one question about them all: null until it
   * answered, or while it cannot, such as while the Cloud is stopped. A
   * render reads it as it is now and renders again when the answer arrives;
   * absent, inline code stays code.
   */
  codeFiles?(paths: readonly string[]): ReadonlySet<string> | null
  /**
   * The conversation's attachment `id`, such as `a3`, as a reply of the
   * agent of `node` shows it: null for the conversation's own agent, a
   * subagent's or a side question's id otherwise. A reply shows only what
   * its own agent uploaded, or the agent it was forked from for the history
   * it copied, so a file the user sent or another agent uploaded is missing
   * there. A render reads it as it is now and renders again when the answer
   * arrives; absent, an attachment shows its alt text.
   */
  attachment?(id: string, node: string | null): AttachmentLookup
  /**
   * Opens attachment `id`, a file that is no image, video or audio, in the
   * work panel through the `attachment` intent, previewed from its copy; absent
   * while no plugin the user has on opens it, as on a share, and its card then
   * only offers Download.
   */
  openAttachment?(id: string, attachment: MessageAttachment): void
  /**
   * Shows a commit a message links as `commit:3f2a9c1`, or as
   * `commit:src/v8@86229da` in the repository of that folder, in the Change
   * view's Commits mode, through the `commit` intent; absent while no plugin the
   * user has on opens it, so such a link shows as text.
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

/** The same for one message, with the working directory its relative paths resolve against. */
export interface MessageFiles extends ConversationFiles {
  cwd: string
  /** The node of the agent that wrote the messages: a subagent's or a side question's id; null or absent, the conversation's own agent. */
  node?: string | null
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
