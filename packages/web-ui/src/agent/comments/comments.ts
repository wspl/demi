import type { MessageEditContent } from '../message-editing'
import type { MessageComment } from '@demicodes/protocol'

/**
 * Comments on a reply's words: each saved comment waits in the conversation's
 * draft until a message sends it, and the reply shows the marks of both, a sent
 * one dimmer, read from the sent messages.
 */

/** A comment the transcript marks: one waiting in the draft, or one a message sent. */
export interface MarkedComment extends MessageComment {
  sent: boolean
}

/**
 * What the transcript's host does with the comments waiting in its draft;
 * absent where a transcript takes none, as a share's page or a subagent's.
 */
export interface ReplyCommenting {
  /** The comments waiting in the draft, in the order they were saved. */
  waiting: readonly MessageComment[]
  /** Saves a new comment, which waits from then on. */
  add: (comment: MessageComment) => void
  /**
   * Send Now: sends a message holding `comment` alone at once, as the
   * composer's send would, steering or waiting while the agent works, and
   * leaves the waiting comments and the typed draft as they are; absent
   * where nothing sends, as a gallery's still specimen.
   */
  sendNow?: (comment: MessageComment) => void
  /** Changes the text of the waiting comment `id`. */
  edit: (id: string, text: string) => void
  /** Removes the waiting comment `id`. */
  remove: (id: string) => void
}

/** The comments a sent message holds, as its `user` or `steer` block keeps them; none for any other block. */
export function sentComments(block: { type: string; comments?: readonly MessageComment[] }): readonly MessageComment[] {
  return block.type === 'user' || block.type === 'steer' ? block.comments ?? [] : []
}

/**
 * `comments` as the model reads them before a message's text: each quoted text
 * as a `>` quote, its comment after it, a blank line between them; empty for
 * none. The page's twin of the backend's `comments_markdown`, which writes them
 * into the model's request.
 */
export function commentsMarkdown(comments: readonly Pick<MessageComment, 'quote' | 'text'>[]): string {
  return comments
    .map(({ quote, text }) => {
      const lines = quote.split('\n').map((line) => line.replace(/\r$/, ''))
      // As Rust's `str::lines`: a final line break ends the last line and starts none.
      if (lines.at(-1) === '') {
        lines.pop()
      }
      return `${lines.map((line) => (line === '' ? '>' : `> ${line}`)).join('\n')}\n\n${text}`
    })
    .join('\n\n')
}

/**
 * `comments` as content before a message's own, as the model reads them: the
 * quoted words as `commentsMarkdown` writes them, and each quoted picture as an
 * image, then its comment. The page's twin of the backend's `comments_content`,
 * by which Copy Message and Send to Conversation take a message's comments, its
 * pictures as files.
 */
export function commentsContent(comments: readonly Pick<MessageComment, 'quote' | 'picture' | 'text'>[]): MessageEditContent[] {
  const content: MessageEditContent[] = []
  let words: Pick<MessageComment, 'quote' | 'text'>[] = []
  const flush = () => {
    if (words.length > 0)
      content.push({ type: 'text', text: commentsMarkdown(words) })
    words = []
  }
  for (const comment of comments) {
    if (!comment.picture) {
      words.push(comment)
      continue
    }
    flush()
    // A blank line after the picture: its comment is a paragraph of its own.
    content.push({ type: 'image', source: comment.picture }, { type: 'text', text: `\n\n${comment.text}` })
  }
  flush()
  return content
}

/**
 * `comments` in the order the conversation reads: by the place of their
 * reply among `blockIds`, then by where each stands in the block's reading
 * order, words and pictures together (`at`); a picture before the words
 * right after it. A comment whose reply the list does not hold goes last,
 * in the order given.
 */
export function inReadingOrder<T extends MessageComment>(comments: readonly T[], blockIds: readonly string[]): T[] {
  const place = new Map(blockIds.map((id, index) => [id, index]))
  const block = (comment: T) => place.get(comment.blockId) ?? blockIds.length
  const words = (comment: T) => (comment.picture ? 0 : 1)
  return [...comments].sort((a, b) => block(a) - block(b) || a.at - b.at || words(a) - words(b))
}
