import { t } from './locales/text'
import type { CredentialFieldKind } from '../permissions/types'
import type { SentenceText } from '../ui/ui-text'
import type { SettingsCredentialDraftField } from './types'

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

/** What breaks the rule of a name, a label or a value, said under its field; null when it holds. */
export function textProblem(text: string): SentenceText | null {
  return text.trim() === '' ? t('credentialFields.required') : null
}

/** What breaks the rule of a value: it is required. */
export function valueProblem(value: string): SentenceText | null {
  return value === '' ? t('credentialFields.required') : null
}

/** A field of the editor, named from its label as the label is typed. */
export interface EditorField {
  /** Tells the fields apart while they are added and removed. */
  key: number
  label: string
  kind: CredentialFieldKind
  value: string
}

/** The names the editor's fields are read by, in order, each made from its label and different from those before it. */
export function fieldNames(fields: readonly EditorField[]): string[] {
  const names: string[] = []
  for (const field of fields) {
    names.push(fieldNameFor(field.label.trim(), names))
  }
  return names
}

/** The editor's fields as the host saves them. */
export function draftFields(fields: readonly EditorField[]): SettingsCredentialDraftField[] {
  const names = fieldNames(fields)
  return fields.map((field, index) => ({
    name: names[index]!,
    label: field.label.trim(),
    kind: field.kind,
    value: field.value,
  }))
}
