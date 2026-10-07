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
  /**
   * A page the agent presented, a tab of its browser, as the command's card
   * names it (`preview.md` § Presenting a page).
   */
  page: PresentedPage
}

/** A page a command presented: the agent's tab, its title and its address. */
export interface PresentedPage {
  tab: string
  title: string
  url: string
}

export type IntentName = keyof IntentPayloads

/** One intent with its payload, as the shell and pages open it. */
export type IntentRequest = { [Name in IntentName]: { intent: Name; payload: IntentPayloads[Name] } }[IntentName]
