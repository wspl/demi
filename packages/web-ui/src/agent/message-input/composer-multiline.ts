/** Expand the composer only when the user has inserted a line break. */
export function composerHasLineBreak(childCount: number, text: string): boolean {
  return childCount > 1 || text.includes('\n')
}
