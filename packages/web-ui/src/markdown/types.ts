export interface MarkdownThemeSnapshot {
  mode: 'light' | 'dark'
  codeThemeId: string
}

/**
 * What the host offers for the files a conversation's messages name on its
 * Host (`file-previews.md` § Files named in messages).
 */
export interface ConversationFiles {
  /** The URL an image at a Host path loads from. */
  imageUrl(path: string): string
  /** Shows the file at a Host path in the work panel's File view. */
  open(path: string): void
}

/** The same for one message, with the working directory its relative paths resolve against. */
export interface MessageFiles extends ConversationFiles {
  cwd: string
}

export interface MarkdownRenderOptions {
  /** Absent, a message's paths stay text and its Host images show their alt text. */
  files?: MessageFiles
}

export type MarkdownRenderer = (src: string, options?: MarkdownRenderOptions) => string
