import { t } from './locales/text'
import type { CredentialFieldKind } from '../permissions/types'
import type { SentenceText } from '../ui/ui-text'
import type { SettingsCredentialDraftField } from './types'

/** The limits of a credential. */
export const NAME_MAX = 80
export const LABEL_MAX = 80
export const FIELD_NAME_MAX = 32
export const FIELDS_MAX = 10
export const VALUE_MAX_BYTES = 64 * 1024

/**
 * The name an agent reads a new field by, made from its label: lowercase letters,
 * digits, `_` and `-`, at most 32, different from every name in `taken`.
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
    .slice(0, FIELD_NAME_MAX - 3)
    .replace(/-+$/, '') || 'field'
  let name = base
  for (let n = 2; used.has(name); n += 1) {
    name = `${base}-${n}`
  }
  return name
}

/** What breaks the rule of a name, a label or a value, said under its field; null when it holds. */
export function textProblem(text: string, max: number): SentenceText | null {
  if (text.trim() === '') {
    return t('credentialFields.required')
  }
  return [...text.trim()].length > max ? t('credentialFields.maxChars', { max }) : null
}

/** What breaks the rule of a value: it is required, and at most 64 KiB. */
export function valueProblem(value: string): SentenceText | null {
  if (value === '') {
    return t('credentialFields.required')
  }
  return new TextEncoder().encode(value).length > VALUE_MAX_BYTES ? t('credentialFields.maxBytes') : null
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
