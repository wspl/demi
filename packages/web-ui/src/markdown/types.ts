/**
 * What the host offers for the files a conversation's messages name on its
 * Host (`file-previews.md` § Files named in messages).
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
}

/** The same for one message, with the working directory its relative paths resolve against. */
export interface MessageFiles extends ConversationFiles {
  cwd: string
}

export interface MarkdownRenderOptions {
  /** Absent, a message's paths stay text and its Host images show their alt text. */
  files?: MessageFiles
}
