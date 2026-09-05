import type { TinybashFs } from '../host'

/** Whether the path is a directory; null when it does not exist (or cannot be stat'ed). */
export async function isDirectory(fs: TinybashFs, cwd: string, path: string): Promise<boolean | null> {
  try {
    return (await fs.stat(path, { cwd })).isDirectory
  } catch {
    return null
  }
}
