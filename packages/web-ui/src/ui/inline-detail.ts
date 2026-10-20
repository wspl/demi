/**
 * What a value stands for, shown after it on its line in smaller, secondary text
 * (InlineDetail). It is one of:
 *
 * - plain, such as an entry's account, or
 * - enclosed in the language's brackets, such as what Automatic resolves to,
 *   `Automatic (Codex)`, whose brackets stay whole while the text inside them
 *   truncates.
 */
export type InlineDetailText = string | { text: string; open: string; close: string }
