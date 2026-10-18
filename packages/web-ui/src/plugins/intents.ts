import type { RequestEditSelection } from '../files/request-changes'

/**
 * What the shell and plugins ask the work panel to show, named by what it
 * shows rather than by who shows it (`plugin-pages.md` § Intents), and the
 * payload of each.
 */
export interface IntentPayloads {
  /**
   * A file on the conversation's Host, by its absolute path, and the line to
   * open it at, scrolled there and marked for a moment, when one is named
   * (`file-previews.md` § Files named in messages); an empty path shows the
   * directory's tree alone. `root` is the directory the view opens for, the
   * working directory of the agent that named the file; absent, the
   * conversation's. `tree`, as Show in Files asks, also shows the view's
   * tree with the file selected and scrolled into view.
   */
  file: { path: string; line?: number; root?: string; tree?: true }
  /**
   * A request's changes, as its line or a file pill under one of its calls
   * names them: the request, its file, and the edit of it or All Changes.
   */
  edit: RequestEditSelection
  /**
   * A commit of the repository a message's agent worked in, by its id or an
   * abbreviation of it, as a `commit:3f2a9c1` link names it
   * (`edit-tracking.md` § Commits mode). `root` is the directory the view
   * opens for, the working directory of the agent that wrote the link;
   * absent, the conversation's.
   */
  commit: { commit: string; root?: string }
  /**
   * An attachment of the conversation (`commands.md` § Attachment
   * commands) that is no image, video or audio, as its block's card opens
   * it (`file-previews.md` § Attachment blocks): its number, its file's
   * name and size in bytes, and where its stored copy loads from.
   */
  attachment: { id: string; name: string; size: number; url: string }
  /**
   * A web page, in a new tab of the conversation's browser, as a link's
   * Open in Work Panel opens it (`web-application.md` § Context menus).
   */
  web: { url: string }
}

export type IntentName = keyof IntentPayloads

/** One intent with its payload, as the shell and pages open it. */
export type IntentRequest = { [Name in IntentName]: { intent: Name; payload: IntentPayloads[Name] } }[IntentName]
