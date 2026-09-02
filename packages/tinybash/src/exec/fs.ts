import type { HostFileSystem } from '@demicodes/shell'

/** Whether the path is a directory; null when it does not exist (or cannot be stat'ed). */
export async function isDirectory(fs: HostFileSystem, cwd: string, path: string): Promise<boolean | null> {
  try {
    return (await fs.stat(path, { cwd })).isDirectory
  } catch {
    return null
  }
}
