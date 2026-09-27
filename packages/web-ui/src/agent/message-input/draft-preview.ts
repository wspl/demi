import { ATTACHMENT_MARK } from '../../markdown/user-markdown'

/**
 * A draft as one line, for an offer to bring it back: its first line that
 * holds anything, with each capsule as its file's name, set apart from the
 * words it stands between as the capsule is, so the user can tell which
 * draft it is.
 */
export function draftPreview(markdown: string, fileNames: readonly string[]): string {
  const names = fileNames[Symbol.iterator]()
  const text = markdown.replaceAll(ATTACHMENT_MARK, (_, offset: number) => {
    const before = offset > 0 && !/\s/u.test(markdown[offset - 1] ?? '') ? ' ' : ''
    const after = /[\p{L}\p{N}]/u.test(markdown[offset + 1] ?? '') ? ' ' : ''
    return `${before}${names.next().value ?? ''}${after}`
  })
  return text.split('\n').map((line) => line.trim()).find((line) => line !== '') ?? ''
}
