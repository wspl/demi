import { t } from './locales/text'
import type { SentenceText } from '../ui/ui-text'

/**
 * The name an agent reads a new field by, made from its label: lowercase letters,
 * digits, `_` and `-`, different from every name in `taken`.
 *
 * Anything else in the label becomes a hyphen. A label with nothing to keep is
 * `field`.
 */
export function fieldNameFor(label: string, taken: Iterable<string>): string {
  const used = new Set(taken)
  const base = label
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .replace(/-+$/, '') || 'field'
  let name = base
  for (let n = 2; used.has(name); n += 1) {
    name = `${base}-${n}`
  }
  return name
}

/** What breaks the rule of a value a request asks for: it is required. */
export function valueProblem(value: string): SentenceText | null {
  return value === '' ? t('credentialFields.required') : null
}
