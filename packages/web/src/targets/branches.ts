/** Validate a local branch name before adding it to the prototype repository. */
export function branchNameError(name: string): string | null {
  if (!name) return 'Enter a branch name.'
  if (
    name === '@' ||
    name === 'HEAD' ||
    name.startsWith('-') ||
    name.endsWith('.') ||
    name.includes('..') ||
    name.includes('@{') ||
    /[\x00-\x20\x7f~^:?*\[\\]/.test(name) ||
    name.split('/').some((part) => !part || part.startsWith('.') || part.endsWith('.lock'))
  )
    return 'Use a valid branch name, such as feature/new-layout.'
  return null
}
