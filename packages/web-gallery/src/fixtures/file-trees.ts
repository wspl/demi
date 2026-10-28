import { FileBrowserError } from '@demicodes/web-ui/files/types'
import type { FileBrowserSource } from '@demicodes/web-ui/files/types'
import type { UploadItem } from '@demicodes/web-ui/files/file-uploads'
import { HostFiles } from '@demicodes/web-ui/files/file-cache'
import { keptSource } from '@demicodes/web-ui/files/kept-source'
import { createMemoryFileSource, dir, file, link, memoryFileReads, textFile, type MemoryDirectory, type MemoryFileSource } from '@demicodes/web-ui/files/memory-source'
import { baseName } from '@demicodes/utils'

/**
 * Projects for the file tree specimens: one with every kind of row, and
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

// A project named as a project is, since its name heads every tree.
export const TREE_ROOT = '/demi'
export const TREE_SELECTED = `${TREE_ROOT}/src/auth/providers/oauth/github/client.ts`

function wrap(root: MemoryDirectory, latencyMs = 0, uploadRate?: number): FileBrowserSource {
  return createMemoryFileSource({
    platform: 'macos',
    home: TREE_ROOT,
    root: dir({ demi: root }),
    latencyMs,
    uploadRate,
  })
}

/** The rows project, answering at once. */
export function rowsSource(): FileBrowserSource {
  return wrap(rowsTree())
}

export const SHOTS_DIR = `${TREE_ROOT}/.claude/screenshots`

/**
 * A folder of 40 screenshots inside `.claude`, with siblings after it: enough
 * rows to scroll it far under the pinned path, and rows left once it folds.
 */
export function screenshotsSource(): FileBrowserSource {
  return wrap(dir({
    '.claude': dir({
      screenshots: dir(stubs(Array.from({ length: 40 }, (_, index) => `screenshot-${String(index + 1).padStart(2, '0')}.png`))),
      'settings.json': file(310, at),
    }),
    src: dir(stubs(['app.ts', 'main.ts'])),
    ...stubs(['README.md', 'package.json', 'tsconfig.json']),
  }))
}

/** The rows project, where listing `slowPath` never finishes. */
export function stuckSource(slowPath: string): FileBrowserSource {
  const reads = memoryFileReads({ platform: 'macos', home: TREE_ROOT, root: dir({ demi: rowsTree() }) })
  return keptSource({
    ...reads,
    list: (path) => path === slowPath ? new Promise(() => {}) : reads.list(path),
  }, { files: new HostFiles() })
}

/** The rows project with `src/auth` refusing and `tests` failing. */
export function failingSource(): FileBrowserSource {
  const root = rowsTree()
  const src = root.children['src'] as MemoryDirectory
  src.children['auth'] = dir({}, { failure: { kind: 'permission', message: 'Permission denied' } })
  root.children['tests'] = dir({}, { failure: { kind: 'other', message: 'Input/output error' } })
  return wrap(root)
}

const MiB = 1024 * 1024

/**
 * The rows project taking uploads at a pace slow enough to watch, with
 * `src/auth` refusing them the way a directory without write access would.
 */
export function uploadSource(): FileBrowserSource {
  const root = rowsTree()
  const src = root.children['src'] as MemoryDirectory
  src.children['auth'] = dir({}, { failure: { kind: 'permission', message: 'Permission denied' } })
  return wrap(root, 0, 8 * MiB)
}

/**
 * A file that says it is `size` bytes without holding them: one small blob
 * referenced as many times as it takes, which a simulated upload never reads.
 */
export function sizedFile(name: string, size: number): File {
  const block = new Blob([new Uint8Array(MiB)])
  const parts: Blob[] = Array.from({ length: Math.floor(size / MiB) }, () => block)
  if (size % MiB > 0)
    parts.push(block.slice(0, size % MiB))
  return new File(parts, name)
}

/** A folder as a drop hands it over: its folders, and its files by path at the sizes given, holding no bytes. */
export function sizedFolder(name: string, directories: string[], files: [path: string, size: number][]): UploadItem {
  return {
    kind: 'folder',
    name,
    directories,
    files: files.map(([path, size]) => ({ path, file: sizedFile(baseName(path), size) })),
  }
}

/** A project that cannot be listed at all. */
export function offlineSource(): FileBrowserSource {
  const inner = wrap(rowsTree())
  return {
    ...inner,
    async list() {
      throw new FileBrowserError('offline', 'The device is offline.')
    },
  }
}

/** Where the link specimens' project points, by its links. */
export const LINKS = {
  folder: `${TREE_ROOT}/vendor`,
  file: `${TREE_ROOT}/config.json`,
  missing: `${TREE_ROOT}/old.log`,
  /** Where `old.log` leads, which the missing specimen puts back. */
  gone: '/tmp/gone.log',
  unreadable: `${TREE_ROOT}/current`,
  /** A file inside the linked folder, reached back through it: the tree unfolds to it at depth. */
  deep: `${TREE_ROOT}/vendor/project/vendor/README.md`,
} as const

/**
 * A project with a symbolic link of each kind: `vendor` to a folder beside
 * it, which holds `project`, a link back to the project; `config.json` to a
 * file in the home's `.config`; `old.log` to a file that is gone; and
 * `current`, a link to itself, whose target cannot be read for the loop.
 */
export function linksSource(): MemoryFileSource {
  return createMemoryFileSource({
    platform: 'macos',
    home: TREE_ROOT,
    root: dir({
      demi: dir({
        src: dir(stubs(['index.ts', 'server.ts'])),
        vendor: link('../shared/vendor', at),
        'config.json': link('/home/alex/.config/app.json', at),
        'old.log': link('/tmp/gone.log', at),
        current: link('current', at),
        'README.md': textFile('# demi\n', at),
      }),
      shared: dir({
        vendor: dir({
          lodash: dir(stubs(['index.js', 'package.json'])),
          project: link('/demi', at),
          'README.md': textFile('# Vendored packages\n', at),
        }),
      }),
      home: dir({
        alex: dir({
          '.config': dir({ 'app.json': textFile('{\n  "theme": "dark",\n  "fontSize": 13\n}\n', at) }),
        }),
      }),
      tmp: dir({}),
    }),
  })
}

/** A project whose names and text hold `retry`: code, its test, a document and a minified bundle of one long line. */
function searchTree(): MemoryDirectory {
  const bundle = Array.from({ length: 12 }, (_, index) =>
    `function r${index}(e){return new Promise(t=>setTimeout(t,e.retryDelay*2**${index}))}`).join(';')
  return dir({
    dist: dir({ 'app.min.js': textFile(`var c={retryDelay:250};${bundle}\n`, at) }),
    docs: dir({ 'network.md': textFile('# Network\n\nA request that fails waits `retryDelay` milliseconds, then twice as long each time.\n', at) }),
    src: dir({
      net: dir({
        'client.ts': textFile([
          'export const retryDelay = 250',
          '',
          'export async function fetchWithRetry(url: string): Promise<Response> {',
          '  for (let attempt = 0; ; attempt++) {',
          '    try {',
          '      return await fetch(url)',
          '    } catch (error) {',
          '      if (attempt === 2)',
          '        throw error',
          '      await sleep(retryDelay * 2 ** attempt)',
          '    }',
          '  }',
          '}',
          '',
        ].join('\n'), at),
        'retry-policy.ts': textFile('export const attempts = 3\n', at),
        'client.test.ts': textFile("import { retryDelay } from './client'\n\ntest('waits before a retry', () => {\n  expect(retryDelay).toBe(250)\n})\n", at),
      }),
      'index.ts': textFile("export { fetchWithRetry } from './net/client'\n", at),
    }),
    'README.md': textFile('# Example\n\nA client that retries.\n', at),
  })
}

/** The search project, answering as a Host does. */
export function searchSource(): FileBrowserSource {
  return wrap(searchTree(), 150)
}

/** The search project on a Host whose search reaches its limit: it stops after what it found, and says so. */
export function limitedSearchSource(): FileBrowserSource {
  const source = wrap(searchTree())
  const search = source.search!
  return {
    ...source,
    async *search(root, text, signal) {
      for await (const event of search(root, text, signal))
        yield event.type === 'end' ? { ...event, limitReached: true } : event
    },
  }
}

/** The search project on a Host whose search never ends, until more typing or closing the field stops it. */
export function searchingSource(): FileBrowserSource {
  return {
    ...wrap(searchTree()),
    async *search(_root, _text, signal) {
      await new Promise((_resolve, reject) => {
        signal.addEventListener('abort', () => reject(signal.reason), { once: true })
      })
    },
  }
}
