import * as fs from 'tinyjs:fs'
import type { HostCwd } from '@demicodes/shell'
import { errnoError, isAbsolutePath, normalizePath } from '@demicodes/utils'

/**
 * A shell's working directory as a validated path: every `chdir` target is
 * checked to be a directory when it is entered, so a shell never holds a
 * cwd that was not there. tinyjs keeps no directory handles; a directory
 * removed after `chdir` fails at the next spawn as `cwd_unusable`.
 */
export async function openRunnerCwd(path: string): Promise<HostCwd> {
  let current = normalizePath(path)
  await assertDirectory(current)
  return {
    get path() {
      return current
    },
    spawnPath: () => current,
    chdir: async (next) => {
      if (next === '.') return
      const target = isAbsolutePath(next) ? normalizePath(next) : normalizePath(`${current}/${next}`)
      await assertDirectory(target)
      current = target
    },
    snapshot: async () => {
      const saved = current
      return {
        restore() {
          current = saved
        },
      }
    },
    close: async () => {},
  }
}

async function assertDirectory(path: string): Promise<void> {
  if ((await fs.stat(path)).kind !== 'dir') {
    throw errnoError('ENOTDIR', `ENOTDIR: not a directory, chdir '${path}'`, { syscall: 'chdir', path })
  }
}
