/**
 * The last part of a Host's path, in either platform's syntax: `/` and `\`
 * both separate, and separators at the end are ignored, so `C:\src\app\`
 * and `/src/app/` both end in `app`. A root has the empty name.
 */
export function baseName(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, '')
  return trimmed.slice(Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\')) + 1)
}
