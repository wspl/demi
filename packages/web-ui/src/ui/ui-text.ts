/**
 * The capitalization style of UI text, as macOS writes it. The gallery's
 * Writing page holds the rule, its sources and its examples. A prop or field
 * that carries UI text names its style through one of the four types below,
 * so whoever writes into it knows which style to follow; they are plain
 * strings and nothing checks them. Text that is content rather than UI (a
 * conversation's title, a file's name, what a model wrote) stays `string`.
 */

/** Title style: a menu item, a button, a tab, a section or group title. */
export type TitleText = string

/** Sentence style: a switch or checkbox label, a settings row label, a tooltip, a description, a message. */
export type SentenceText = string

/**
 * A headline that is either a fragment or a sentence: an alert's, a toast's
 * or a dialog's title, or what an empty list says. Title style for a fragment
 * ("Rename Failed", "No Results"), sentence style with its ending punctuation
 * for a complete sentence ("Remove this project?").
 */
export type HeadlineText = string

/**
 * A field's hint: sentence style without ending punctuation ("Search
 * conversations"), or an example value written as the person would type it
 * ("name@example.com").
 */
export type PlaceholderText = string
