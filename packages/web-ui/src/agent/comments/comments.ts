import { z } from 'zod'
import { messageCommentSchema, type MediaSource, type MessageComment } from '@demicodes/protocol'
import { ATTACHMENT_MARK } from '../../markdown/user-markdown'
import type { PasteHost } from '../message-input/copied-message'
import { contentMessage, type MessageCapsule, type TransferContext } from '../message-editor/capsules'

/**
 * Comments on a reply's words or pictures. A comment is a message beside the
 * composer's, with the place it stands on: the composer's draft holds its own
 * message and the comments waiting with it, each written the same way, its
 * files uploaded the same way, and Send sends them together as one message,
 * the comments before the text. The reply shows the marks of the waiting and
 * the sent ones, a sent one dimmer.
 */

/**
 * A message as the page writes it, the composer's or a comment's: its
 * Markdown with an attachment mark where each file's capsule stands, and the
 * ids of its files among the conversation's, in the order of their marks.
 */
export const draftMessageSchema = z.object({
  text: z.string(),
  fileIds: z.array(z.string()),
})
export type DraftMessage = z.infer<typeof draftMessageSchema>

/** A comment waiting in the draft: where it stands, and its message. */
export const waitingCommentSchema = messageCommentSchema.omit({ content: true }).extend({
  content: draftMessageSchema,
})
export type WaitingComment = z.infer<typeof waitingCommentSchema>

/** Where a comment stands: the reply's words or picture it quotes. */
export type CommentAnchor = Omit<MessageComment, 'content' | 'id'>

/**
 * A comment the transcript marks: one waiting in the draft, or one a message
 * sent.
 */
export interface MarkedComment extends ShownComment {
  sent: boolean
}

/**
 * The files of the messages a conversation's page writes, the composer's and
 * its comments': a comment's editor takes files the way the composer does,
 * among the conversation's.
 */
export interface MessageFiles {
  /** Takes picked, pasted or dropped files, uploading each; answers their capsules. */
  add: (files: File[]) => MessageCapsule[]
  /** The capsule of each file of `ids` the conversation holds, in their order. */
  capsules: (ids: readonly string[]) => MessageCapsule[]
  /** The transfers of those files, which their capsules show. */
  transfers: TransferContext
  /** How a copied message pasted into the editor brings its files back. */
  paste: PasteHost
  /**
   * The files that `editor`, an open editor or a message on its way, holds
   * now, which stay whatever else lets files go; null once it is closed or
   * sent, which lets each file it took go that no message holds.
   */
  hold: (editor: string, ids: readonly string[] | null) => void
}

/**
 * What the transcript's host does with the comments waiting in its draft;
 * absent where a transcript takes none, as a share's page or a subagent's.
 */
export interface ReplyCommenting {
  /** The comments waiting in the draft, in the order they were saved. */
  waiting: readonly WaitingComment[]
  /** The files the comments and their editors hold. */
  files: MessageFiles
  /** Saves a new comment, which waits from then on. */
  add: (comment: WaitingComment) => void
  /**
   * Steer: sends a message holding `comment` alone at once, as the
   * composer's send would, steering or waiting while the agent works. It leaves
   * the waiting comments and the typed draft as they are. Absent where nothing
   * sends, as a gallery's still specimen.
   */
  sendNow?: (comment: WaitingComment) => void
  /** Changes the message of the waiting comment `id`. */
  edit: (id: string, content: DraftMessage) => void
  /** Removes the waiting comment `id`. */
  remove: (id: string) => void
}

/** Whether a message says something: words, or a file. */
export function saysSomething(message: DraftMessage): boolean {
  return message.fileIds.length > 0 || message.text.replaceAll(ATTACHMENT_MARK, '').trim() !== ''
}

/**
 * The comments a sent message holds, as its `user` or `steer` block keeps them;
 * none for any other block.
 */
export function sentComments(block: { type: string; comments?: readonly MessageComment[] }): readonly MessageComment[] {
  return block.type === 'user' || block.type === 'steer' ? block.comments ?? [] : []
}

/**
 * A comment as a message's bubble shows it: its quote, and its message as
 * Markdown with a capsule for each file.
 */
export interface ShownComment extends Omit<MessageComment, 'content'> {
  markdown: string
  attachments: readonly MessageCapsule[]
}

/** A sent comment as its bubble shows it. */
export function shownComment({ content, ...comment }: MessageComment): ShownComment {
  const { markdown, capsules } = contentMessage(content)
  return { ...comment, markdown, attachments: capsules }
}

/**
 * `comments` as one message before a message's own, as the model reads them:
 * each quoted text as a `>` quote, or each quoted picture as its capsule,
 * then the comment's message, a blank line between each. The page's twin of
 * the backend's `comments_content`, by which Copy Message and Send to
 * Conversation take a message's comments, their files as files.
 */
export function commentsMessage(comments: readonly ShownComment[]): { markdown: string; capsules: MessageCapsule[] } {
  const parts: string[] = []
  const capsules: MessageCapsule[] = []
  for (const comment of comments) {
    if (comment.picture) {
      const picture = contentMessage([{ type: 'image', source: comment.picture }])
      parts.push(picture.markdown)
      capsules.push(...picture.capsules)
    } else {
      parts.push(quoted(comment.quote))
    }
    parts.push(comment.markdown)
    capsules.push(...comment.attachments)
  }
  return { markdown: parts.filter((part) => part !== '').join('\n\n'), capsules }
}

/** `text` as a Markdown quote, each line after `> `, an empty one as `>`. */
function quoted(text: string): string {
  const lines = text.split('\n').map((line) => line.replace(/\r$/, ''))
  // As Rust's `str::lines`: a final line break ends the last line and starts
  // none.
  if (lines.at(-1) === '')
    lines.pop()
  return lines.map((line) => (line === '' ? '>' : `> ${line}`)).join('\n')
}

/**
 * `comments` in the order the conversation reads: by the place of their reply
 * among `blockIds`, then by where each stands in the block's reading order,
 * words and pictures together (`at`). A picture comes before the words right
 * after it. A comment whose reply the list does not hold goes last, in the
 * order given.
 */
export function inReadingOrder<T extends CommentAnchor>(comments: readonly T[], blockIds: readonly string[]): T[] {
  const place = new Map(blockIds.map((id, index) => [id, index]))
  const block = (comment: T) => place.get(comment.blockId) ?? blockIds.length
  const words = (comment: T) => (comment.picture ? 0 : 1)
  return [...comments].sort((a, b) => block(a) - block(b) || a.at - b.at || words(a) - words(b))
}
