import type { CredentialFieldKind } from '../permissions/types'
import type { SentenceText } from '../ui/ui-text'
import type { SettingsCredentialDraftField } from './types'

/** The limits of a credential (`web-api.md` § Credentials). */
export const NAME_MAX = 80
export const LABEL_MAX = 80
export const FIELD_NAME_MAX = 32
export const FIELDS_MAX = 10
export const VALUE_MAX_BYTES = 64 * 1024

/**
 * The name an agent reads a new field by, made from its label: lowercase
 * letters, digits, `_` and `-`, at most 32, different from every name in
 * `taken`. Anything else in the label becomes a hyphen; a label with
 * nothing to keep is `field`.
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
    return 'Required'
  }
  return [...text.trim()].length > max ? `Use at most ${max} characters.` : null
}

/** What breaks the rule of a value's size; a value may be empty only where the field keeps its saved one. */
export function valueProblem(value: string, keepsSaved: boolean): SentenceText | null {
  if (value === '') {
    return keepsSaved ? null : 'Required'
  }
  return new TextEncoder().encode(value).length > VALUE_MAX_BYTES ? 'Use at most 64 KiB.' : null
}

/** A field of the editor: a saved field keeps its name, a new one is named from its label as the label is typed. */
export interface EditorField {
  /** Tells the fields apart while they are added and removed. */
  key: number
  /** The name of a saved field; null for a new one. */
  saved: string | null
  label: string
  kind: CredentialFieldKind
  value: string
}

/** The names the editor's fields are read by, in order: a saved field's own, a new field's made from its label. */
export function fieldNames(fields: readonly EditorField[]): string[] {
  const names: string[] = fields.flatMap((field) => (field.saved === null ? [] : [field.saved]))
  return fields.map((field) => {
    if (field.saved !== null) {
      return field.saved
    }
    const name = fieldNameFor(field.label.trim(), names)
    names.push(name)
    return name
  })
}

/** The editor's fields as the host saves them: an empty value keeps the saved one. */
export function draftFields(fields: readonly EditorField[]): SettingsCredentialDraftField[] {
  const names = fieldNames(fields)
  return fields.map((field, index) => ({
    name: names[index]!,
    label: field.label.trim(),
    kind: field.kind,
    value: field.value === '' && field.saved !== null ? null : field.value,
  }))
}
