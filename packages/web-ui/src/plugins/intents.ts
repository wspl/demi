import type { CallEditSelection } from '../files/changes'

/**
 * What the shell and plugins ask the work panel to show, named by what it
 * shows rather than by who shows it (`plugin-pages.md` § Intents), and the
 * payload of each.
 */
export interface IntentPayloads {
  /** A file on the conversation's Host, by its absolute path. */
  file: { path: string }
  /** One call's edit of one file, as the transcript's tool block names it. */
  edit: CallEditSelection
}

export type IntentName = keyof IntentPayloads

/** One intent with its payload, as the shell and pages open it. */
export type IntentRequest = { [Name in IntentName]: { intent: Name; payload: IntentPayloads[Name] } }[IntentName]
