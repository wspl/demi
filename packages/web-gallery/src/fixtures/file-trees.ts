import { FileBrowserError } from '@demicodes/web-ui/files/types'
import type { FileBrowserSource } from '@demicodes/web-ui/files/types'
import { createMemoryFileSource, dir, file, type MemoryDirectory } from '@demicodes/web-ui/files/memory-source'

/**
 * Workspaces for the file tree specimens: one with every kind of row, and
 * sources that never answer, refuse, or fail, so each state can be seen at
 * rest. Nothing here reads a real disk.
 */
const at = new Date(Date.UTC(2026, 8, 12, 9, 30)).toISOString()

function stubs(names: string[]): Record<string, ReturnType<typeof file>> {
  return Object.fromEntries(names.map((name) => [name, file(120, at)]))
}

/** Every kind of row: open and closed directories, files, hidden entries, deep nesting, long names. */
function rowsTree(): MemoryDirectory {
  return dir({
    '.git': dir(stubs(['HEAD', 'config'])),
    '.gitignore': file(30, at),
    'README.md': file(2_048, at),
    'package.json': file(1_244, at),
    'a-very-long-directory-name-that-truncates': dir(stubs(['notes.md'])),
    docs: dir(stubs(['overview.md', 'roadmap.md'])),
    src: dir({
      auth: dir({
        ...stubs(['cookie.ts', 'password.ts', 'session.ts']),
        providers: dir({
          oauth: dir({
            github: dir(stubs(['callback.ts', 'client.ts', 'a-very-long-file-name-that-truncates.ts'])),
          }),
        }),
      }),
      http: dir(stubs(['middleware.ts', 'router.ts', 'server.ts'])),
      'index.ts': file(40, at),
    }),
    tests: dir(stubs(['auth.test.ts', 'http.test.ts'])),
    // Enough after src for the tree to scroll it fully out of view.
    scripts: dir(stubs(['build.ts', 'dev.sh', 'release.ts'])),
    ...stubs(['bunfig.toml', 'tsconfig.json', 'vite.config.ts', 'LICENSE', 'CHANGELOG.md', 'CONTRIBUTING.md']),
  })
}

export const TREE_ROOT = '/w'
export const TREE_SELECTED = `${TREE_ROOT}/src/auth/providers/oauth/github/client.ts`

function wrap(root: MemoryDirectory, latencyMs = 0): FileBrowserSource {
  return createMemoryFileSource({
    platform: 'macos',
    home: '/w',
    root: dir({ w: root }),
    latencyMs,
  })
}

/** The rows workspace, answering at once. */
export function rowsSource(): FileBrowserSource {
  return wrap(rowsTree())
}

/** The rows workspace, where listing `slowPath` never finishes. */
export function stuckSource(slowPath: string): FileBrowserSource {
  const inner = wrap(rowsTree())
  return {
    ...inner,
    list(path, signal) {
      if (path === slowPath) {
        return new Promise((_resolve, reject) => {
          signal?.addEventListener('abort', () => reject(new DOMException('Aborted', 'AbortError')), { once: true })
        })
      }
      return inner.list(path, signal)
    },
  }
}

/** The rows workspace with `src/auth` refusing and `tests` failing. */
export function failingSource(): FileBrowserSource {
  const root = rowsTree()
  const src = root.children['src'] as MemoryDirectory
  src.children['auth'] = dir({}, { failure: { kind: 'permission', message: 'Permission denied' } })
  root.children['tests'] = dir({}, { failure: { kind: 'other', message: 'Input/output error' } })
  return wrap(root)
}

/** A workspace that cannot be listed at all. */
export function offlineSource(): FileBrowserSource {
  const inner = wrap(rowsTree())
  return {
    ...inner,
    async list() {
      throw new FileBrowserError('offline', 'The device is offline.')
    },
  }
}
