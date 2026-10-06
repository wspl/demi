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
 * What the host offers for the files a conversation's messages name on its
 * Host and the attachments they name (`file-previews.md` § Files named in
 * messages).
 */
export interface ConversationFiles {
  /** The URL an image at a Host path loads from. */
  imageUrl(path: string): string
  /**
   * Shows the file at a Host path, through the `file` intent; absent while no
   * plugin the user has on opens it, so a link to a file shows as text and a
   * Host image only shows (`file-previews.md` § Files named in messages).
   */
  open?(path: string): void
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
}
