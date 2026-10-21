import { nonEmptyString, truncate } from '@demicodes/utils'
import { t } from './locales/text'

/**
 * How a call renders: the one tool, `shell`, has its own row; a call of
 * any other name, such as a `shell_status` or a `yield` a transcript kept
 * from before they were removed, is a generic tool card.
 */
export type ToolRenderKind = 'shell' | 'generic'

export function toolRenderKind(toolName: string): ToolRenderKind {
  return toolName === 'shell' ? 'shell' : 'generic'
}

/**
 * A `shell` call's title: its description, or its script for a call that has
 * none.
 */
export function shellTitle(input: Record<string, unknown>): string {
  return nonEmptyString(input.description) ?? nonEmptyString(input.script) ?? t('transcript.shellFallback')
}

/**
 * The options of `demi shell status` that take a value, as the next word or
 * after `=`.
 */
const STATUS_VALUE_OPTIONS = new Set(['--interval'])
/** The options of `demi shell status` that take no value. */
const STATUS_FLAGS = new Set(['--resident', '--json'])

/**
 * The commands a script looks at when all it runs is `demi shell status`,
 * by their numbers, each once; null for a script that does anything else,
 * which runs a command of its own. Commands joined by a new line, `;`,
 * `&&` or `||` are each a look; a pipe, a redirection or a substitution
 * makes the script more than a look, and so does an option status does
 * not take, such as one an earlier release took.
 */
export function shellStatusLooks(script: string): string[] | null {
  const looked = new Set<string>()
  const commands = script
    .split('\n')
    .filter((line) => !line.trim().startsWith('#'))
    .flatMap((line) => line.split(/&&|\|\||;/))
    .map((command) => command.trim())
    .filter((command) => command !== '')
  if (commands.length === 0)
    return null
  for (const command of commands) {
    if (/[|<>$`()]/.test(command))
      return null
    const [demi, shell, status, ...words] = command.split(/\s+/)
    if (demi !== 'demi' || shell !== 'shell' || status !== 'status')
      return null
    for (let at = 0; at < words.length; at += 1) {
      const word = words[at]!
      if (/^\d+$/.test(word))
        looked.add(word)
      else if (STATUS_VALUE_OPTIONS.has(word))
        at += 1
      else if (!STATUS_FLAGS.has(word) && !STATUS_VALUE_OPTIONS.has(word.split('=')[0]!))
        return null
    }
  }
  return looked.size > 0 ? [...looked] : null
}

export function trimToolSummary(text: string, maxLength = 120): string {
  return truncate(text.replace(/\s+/g, ' ').trim(), maxLength, '...')
}
