import type { RequestEditSelection } from '../files/request-changes'

/**
 * What the shell and plugins ask the work panel to show, named by what it
 * shows rather than by who shows it, and the
 * payload of each.
 */
export interface IntentPayloads {
  /**
   * A file on the conversation's Host, by its absolute path, and the line to open
   * it at, scrolled there and marked for a moment, when one is named. An empty
   * path shows the directory's tree alone.
   *
   * - `root` is the directory the view opens for, the working directory of the
   *   agent that named the file. Absent, the conversation's.
   * - `tree`, as Show in Files asks, also shows the view's tree with the file
   *   selected and scrolled into view.
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
   * (`packages/web-ui/src/files/history.ts`).
   *
   * `root` is the directory the view opens for, the working directory of the agent
   * that wrote the link; absent, the conversation's.
   */
  commit: { commit: string; root?: string }
  /**
   * An attachment of the conversation (`crates/backend-host-access/src/attachment_commands.rs`) that is no image, video or audio, as its block's card opens
   * it: its number, its file's name and size in bytes, and where its stored
   * copy loads from.
   */
  attachment: { id: string; name: string; size: number; url: string }
  /**
   * The medium the conversation's media viewer shows now
   * (`packages/web-ui/src/files/media-viewer.ts`), in the work panel's one
   * Media tab, as opening a picture or a video of the conversation shows it.
   * The viewer holds the medium and its series, so the request names nothing.
   */
  media: Record<string, never>
  /**
   * A web page, in a new tab of the conversation's browser, as a link's
   * Open in Work Panel opens it.
   */
  web: { url: string }
}

export type IntentName = keyof IntentPayloads

/** One intent with its payload, as the shell and pages open it. */
export type IntentRequest = { [Name in IntentName]: { intent: Name; payload: IntentPayloads[Name] } }[IntentName]
