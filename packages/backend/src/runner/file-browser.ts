import type { HostFileSystem } from '@demicodes/shell'
import { errorCode } from '@demicodes/utils'

/**
 * File metadata comes from the selected runner. A disappeared entry is omitted
 * from this directory snapshot.
 */
export async function browseDirectory(fs: HostFileSystem, path: string) {
  const entries = await fs.readdir(path, { withFileTypes: true })
  const result = []
  for (let index = 0; index < entries.length; index += 32) {
    const batch = await Promise.all(
      entries.slice(index, index + 32).map(
        async entry => {
          const absolute = `${path.replace(/\/+$/, '')}/${entry.name}`
          try {
            const stat = await fs.lstat(absolute)
            return {
              name: entry.name,
              isDirectory: entry.isDirectory,
              isSymbolicLink: stat.isSymbolicLink,
              size: stat.size,
              modifiedAt: stat.mtime.toISOString()
            }
          } catch (error) {
            if (errorCode(error) === 'ENOENT')
              return null
            throw error
          }
        }
      )
    )
    result.push(...batch.filter(entry => entry !== null))
  }
  return result
}
