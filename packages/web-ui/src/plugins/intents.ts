import type { RequestEditSelection } from '../files/request-changes'

/**
 * What the shell and plugins ask the work panel to show, named by what it
 * shows rather than by who shows it (`plugin-pages.md` § Intents), and the
 * payload of each.
 */
export interface IntentPayloads {
  /** A file on the conversation's Host, by its absolute path. */
  file: { path: string }
  /**
   * A request's changes, as its line or a file pill under one of its calls
   * names them: the request, its file, and the edit of it or All Changes.
   */
  edit: RequestEditSelection
}

export type IntentName = keyof IntentPayloads

/** One intent with its payload, as the shell and pages open it. */
export type IntentRequest = { [Name in IntentName]: { intent: Name; payload: IntentPayloads[Name] } }[IntentName]
