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

/**
 * How a page opens an intent: the pinned kind whose tab shows it, and the
 * data that tab shows next, from the payload and the data it shows now, or
 * null while it has shown nothing yet in this page.
 */
export interface IntentTarget<Name extends IntentName> {
  kind: string
  open(payload: IntentPayloads[Name], current: unknown): unknown
}

/** The intents one page opens. */
export type PageIntents = { [Name in IntentName]?: IntentTarget<Name> }
