import { reactive } from 'vue'
import type { ChangeFile } from '@demicodes/web-ui/files/changes'
import type { ChangeSetSource } from '@demicodes/web-ui/files/changes'
import { assetFile, createMemoryFileSource, dir, textFile, type MemoryDirectory } from '@demicodes/web-ui/files/memory-source'
import { FileBrowserError, type FileBrowserSource, type FileContents } from '@demicodes/web-ui/files/types'

/**
 * The demo workspace behind the work panel: the project the login-test
 * conversation edits, with the files its tabs and changed-file pills name.
 * Nothing here reads a real disk.
 */
export const WORKSPACE_ROOT = '/Users/zan/Projects/demi'

const at = new Date(Date.UTC(2026, 8, 12, 9, 30)).toISOString()

const cookieTs = `import { parse, serialize } from 'cookie'

/** The session cookie: renamed from \`sid\` so the old name stops working. */
export const SESSION_COOKIE = 'session'

export function readSession(header: string | undefined): string | null {
  if (!header) {
    return null
  }
  return parse(header)[SESSION_COOKIE] ?? null
}

export function writeSession(id: string): string {
  return serialize(SESSION_COOKIE, id, {
    httpOnly: true,
    sameSite: 'lax',
    path: '/',
  })
}
`

const sessionTs = `import { readSession } from './cookie'

export interface Session {
  id: string
  userId: string
  expiresAt: Date
}

const sessions = new Map<string, Session>()

export function sessionFromHeader(header: string | undefined): Session | null {
  const id = readSession(header)
  if (!id) {
    return null
  }
  const session = sessions.get(id)
  if (!session || session.expiresAt < new Date()) {
    return null
  }
  return session
}
`

const authTest = `import { describe, expect, test } from 'bun:test'
import { readSession, writeSession, SESSION_COOKIE } from '../../src/auth/cookie'

describe('session cookie', () => {
  test('writes the session cookie', () => {
    expect(writeSession('abc')).toContain(\`\${SESSION_COOKIE}=abc\`)
  })

  test('reads the session cookie', () => {
    expect(readSession('session=abc; theme=dark')).toBe('abc')
  })

  test('reads nothing without a header', () => {
    expect(readSession(undefined)).toBeNull()
  })
})
`

const readme = `---
title: demi
tags: [agents, cloud]
---

<p align="center"><img src="assets/photo.png" width="240" alt="The test pattern"></p>

# demi

Conversations that run commands on Cloud or your own machines.

- [Getting started](#getting-started)
- [The guide](docs/guides/getting-started.md), and [the session cookie](/src/auth/cookie.ts)
- [Bun](https://bun.sh), which runs all of it, and [why a Cloud](#why-a-cloud)

## Getting started

\`\`\`bash
bun install
bun run dev
\`\`\`

| Command | What it does |
| :-- | :-: |
| \`bun run dev\` | Starts the product |
| \`bun run typecheck\` | Checks every package |

- [x] Previews images, video, audio and PDF
- [ ] Mermaid diagrams

The identity $e^{i\\pi} + 1 = 0$, and a block:

$$
\\int_0^1 x^2\\,dx = \\frac{1}{3}
$$

<a name="why-a-cloud"></a>
<details><summary>Why a Cloud?</summary>

Work keeps running while the laptop sleeps.

</details>

<script>alert('a document never runs script')</script>
<img src="assets/missing.png" onerror="alert('nor handlers')" alt="A missing image">
`

const logoSvg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 120 120" width="120" height="120">
  <rect x="10" y="10" width="100" height="100" rx="24" fill="#3b82f6"/>
  <circle cx="60" cy="60" r="26" fill="#ffffff"/>
</svg>
`

const logoSvgBefore = logoSvg.replace('#3b82f6', '#f97316').replace('rx="24"', 'rx="8"')

const packageJson = `{
  "name": "demi",
  "private": true,
  "workspaces": ["packages/*"],
  "scripts": {
    "dev": "bun run --filter '*' dev",
    "typecheck": "tsgo --noEmit"
  }
}
`

const pillsVue = `<script setup lang="ts">
import type { ChangeFile } from '../../files/changes'

defineProps<{ files: ChangeFile[] }>()
</script>

<template>
  <div class="flex flex-wrap gap-1">
    <span v-for="file in files" :key="file.path">{{ file.path }}</span>
  </div>
</template>
`

/** A source file with a short body naming itself, so every tab shows something. */
function stub(path: string, body = ''): ReturnType<typeof textFile> {
  const name = path.slice(path.lastIndexOf('/') + 1)
  const ext = name.slice(name.lastIndexOf('.') + 1)
  const header = ext === 'md'
    ? `# ${name.replace(/\.md$/, '')}\n`
    : ext === 'json'
      ? '{\n'
      : ext === 'vue'
        ? `<script setup lang="ts">\n// ${path}\n</script>\n\n<template>\n  <div />\n</template>\n`
        : `// ${path}\n`
  const footer = ext === 'json' ? `  "name": "${name}"\n}\n` : ''
  return textFile(header + body + footer, at)
}

/** A directory of stub files, listed by name. */
function files(base: string, names: string[]): Record<string, ReturnType<typeof textFile>> {
  return Object.fromEntries(names.map((name) => [name, stub(`${base}/${name}`)]))
}

function tree(): MemoryDirectory {
  return dir({
    '.git': dir({ HEAD: textFile('ref: refs/heads/main\n', at), config: stub('.git/config') }),
    '.github': dir({
      workflows: dir(files('.github/workflows', ['ci.yml', 'release.yml', 'docs.yml'])),
      'CODEOWNERS': stub('.github/CODEOWNERS'),
    }),
    '.vscode': dir(files('.vscode', ['settings.json', 'extensions.json'])),
    '.gitignore': textFile('node_modules\ndist\n', at),
    // Files the work panel previews rather than reads as text (`file-previews.md`).
    assets: dir({
      'logo.svg': textFile(logoSvg, at),
      'photo.png': assetFile('/fixtures/preview/photo.png', 15822, at),
      'page-full.png': assetFile('/fixtures/preview/page-full.png', 8035, at),
      'demo.mp4': assetFile('/fixtures/preview/demo.mp4', 31814, at),
      'intro.mov': assetFile('/fixtures/preview/demo.mp4', 31814, at),
      'tone.m4a': assetFile('/fixtures/preview/tone.m4a', 10396, at),
    }),
    dist: dir({
      'app.zip': assetFile('/fixtures/preview/app.zip', 140, at),
      'cache.db': assetFile('/fixtures/preview/app.zip', 140, at),
    }),
    '.editorconfig': stub('.editorconfig'),
    'AGENTS.md': stub('AGENTS.md'),
    'CLAUDE.md': stub('CLAUDE.md'),
    'README.md': textFile(readme, at),
    'bunfig.toml': stub('bunfig.toml'),
    'package.json': textFile(packageJson, at),
    'tsconfig.json': stub('tsconfig.json'),
    docs: dir({
      ...files('docs', [
        'agent-messages.md', 'package-boundaries.md', 'session-storage-and-naming.md',
        'tool-rendering-spec.md', 'web-authentication.md', 'web-integration.md',
      ]),
      'demi-next': dir(files('docs/demi-next', [
        'backend.md', 'overview.md', 'product.md', 'roadmap.md', 'runner.md',
        'sessions-and-targets.md', 'storage.md', 'web-application.md',
      ])),
      guides: dir(files('docs/guides', ['getting-started.md', 'deploying.md'])),
      'guide.pdf': assetFile('/fixtures/preview/guide.pdf', 734, at),
    }),
    packages: dir({
      agent: dir({
        src: dir({
          ...files('packages/agent/src', ['agent.ts', 'client.ts', 'index.ts', 'tools.ts', 'types.ts']),
          providers: dir(files('packages/agent/src/providers', ['anthropic.ts', 'openai.ts', 'index.ts'])),
        }),
        'package.json': stub('packages/agent/package.json'),
      }),
      core: dir({
        src: dir(files('packages/core/src', ['blocks.ts', 'index.ts', 'messages.ts', 'schemas.ts'])),
        'package.json': stub('packages/core/package.json'),
      }),
      web: dir({
        src: dir({
          ...files('packages/web/src', ['App.vue', 'main.ts', 'style.css']),
          auth: dir(files('packages/web/src/auth', ['session.ts', 'LoginPage.vue'])),
          conversation: dir(files('packages/web/src/conversation', ['ChatPage.vue', 'ConversationComposer.vue', 'store.ts'])),
          state: dir(files('packages/web/src/state', ['local.ts', 'resources.ts', 'types.ts'])),
        }),
        'index.html': stub('packages/web/index.html'),
        'package.json': stub('packages/web/package.json'),
        'vite.config.ts': stub('packages/web/vite.config.ts'),
      }),
      'web-ui': dir({
        src: dir({
          agent: dir({
            ...files('packages/web-ui/src/agent', [
              'AgentMessageList.vue', 'ChatSession.vue', 'SessionDock.vue', 'SessionSurface.vue',
              'TabItem.vue', 'TabStrip.vue', 'WorkPanel.vue', 'block-helpers.ts', 'tab-close.ts',
              'tab-strip.ts', 'work-panel.ts',
            ]),
            blocks: dir(files('packages/web-ui/src/agent/blocks', [
              'AssistantBlock.vue', 'ErrorBlock.vue', 'FileChangePills.vue', 'FunctionalBlock.vue',
              'ThinkingBlock.vue', 'ToolShellBlock.vue', 'UserBlock.vue',
            ])),
            __tests__: dir(files('packages/web-ui/src/agent/__tests__', ['block-helpers.test.ts', 'work-panel.test.ts'])),
          }),
          files: dir(files('packages/web-ui/src/files', [
            'FileBrowser.vue', 'FileBrowserAddressBar.vue', 'FileIcon.vue', 'FileTree.vue', 'FileView.vue',
            'memory-source.ts', 'paths.ts', 'types.ts',
          ])),
          ui: dir(files('packages/web-ui/src/ui', [
            'Button.vue', 'Dialog.vue', 'Dropdown.vue', 'IconButton.vue', 'Menu.vue', 'MenuItem.vue',
            'Popover.vue', 'ResizeHandle.vue', 'ScrollArea.vue', 'TextInput.vue', 'Tooltip.vue',
            'icon-metrics.ts', 'resize-handle.ts',
          ])),
          styles: dir(files('packages/web-ui/src/styles', ['base.css', 'product-appearance.css'])),
          theme: dir(files('packages/web-ui/src/theme', ['appTheme.ts', 'codeTheme.ts', 'themeStore.ts'])),
        }),
        'package.json': stub('packages/web-ui/package.json'),
      }),
    }),
    scripts: dir(files('scripts', ['check-versions.ts', 'dev.sh', 'release.ts'])),
    src: dir({
      auth: dir({
        'cookie.ts': textFile(cookieTs, at),
        'session.ts': textFile(sessionTs, at),
        'password.ts': stub('src/auth/password.ts'),
      }),
      http: dir(files('src/http', ['router.ts', 'server.ts', 'middleware.ts'])),
      'index.ts': textFile("export * from './auth/session'\n", at),
    }),
    tests: dir({
      login: dir({
        'auth.test.ts': textFile(authTest, at),
        'password.test.ts': stub('tests/login/password.test.ts'),
      }),
      http: dir(files('tests/http', ['router.test.ts', 'server.test.ts'])),
      fixtures: dir(files('tests/fixtures', ['users.json', 'sessions.json'])),
    }),
  })
}

/** Lines added and removed between two texts, by a longest common subsequence of lines. */
function lineCounts(original: string, modified: string): { added: number; removed: number } {
  const a = original === '' ? [] : original.split('\n')
  const b = modified === '' ? [] : modified.split('\n')
  const table: number[][] = Array.from({ length: a.length + 1 }, () => new Array<number>(b.length + 1).fill(0))
  for (let i = a.length - 1; i >= 0; i--) {
    for (let j = b.length - 1; j >= 0; j--) {
      table[i]![j] = a[i] === b[j] ? table[i + 1]![j + 1]! + 1 : Math.max(table[i + 1]![j]!, table[i]![j + 1]!)
    }
  }
  const common = table[0]![0]!
  return { added: b.length - common, removed: a.length - common }
}

/** A generated file of about `lines` lines in the language its extension names, so a diff has room for several hunks. */
function generated(path: string, lines: number, seed = 1): string {
  const name = path.slice(path.lastIndexOf('/') + 1)
  const ext = name.slice(name.lastIndexOf('.') + 1)
  const ident = name.replace(/\.[^.]+$/, '').replace(/[^a-zA-Z0-9]/g, '_')
  const script = (): string[] => {
    const out = [`export const ${ident}Version = ${seed}`, '']
    for (let i = 1; i <= lines; i++) {
      if (i % 12 === 1) {
        out.push(`export function step${i}(input: number): number {`)
      }
      out.push(`  const value${i} = input * ${i} + ${seed}`)
      if (i % 12 === 0 || i === lines) {
        out.push(`  return value${i}`, '}', '')
      }
    }
    return out
  }
  switch (ext) {
    case 'vue': {
      const items = Array.from({ length: Math.ceil(lines / 6) }, (_, i) => `    <li :key="${i}">Item ${i + 1} of ${seed}</li>`)
      return [
        '<script setup lang="ts">',
        `// ${path}`,
        ...script().map((line) => line),
        '</script>',
        '',
        '<template>',
        '  <ul class="flex flex-col gap-1">',
        ...items,
        '  </ul>',
        '</template>',
        '',
      ].join('\n')
    }
    case 'md': {
      const out = [`# ${ident}`, '']
      for (let i = 1; i <= lines; i++) {
        if (i % 8 === 1) {
          out.push(`## Section ${i}`, '')
        }
        out.push(`Paragraph ${i} of the design, revision ${seed}. It explains step ${i} and why it holds.`)
        if (i % 8 === 0) {
          out.push('', '```ts', `const step${i} = ${seed}`, '```', '')
        }
      }
      return out.join('\n') + '\n'
    }
    case 'yml': {
      const out = [`name: ${ident}`, 'on:', '  push:', '    branches: [main]', 'jobs:', '  build:', '    runs-on: ubuntu-latest', '    steps:']
      for (let i = 1; i <= lines; i++) {
        out.push(`      - name: Step ${i}`, `        run: bun run step${i} -- --seed ${seed} + ${i}`)
      }
      return out.join('\n') + '\n'
    }
    default:
      return [`// ${path}`, '', ...script()].join('\n') + '\n'
  }
}

/** `text` with a few edits spread through it: lines changed, a block added, a block removed. */
function edited(text: string, seed: number): string {
  const lines = text.split('\n')
  const out: string[] = []
  // The inserted block in the file's own idiom: a comment and a statement, a paragraph, or a step.
  const code = /^\s*(const|export|return|import) /m.test(text)
  const inserted = (i: number): string[] =>
    code
      ? [`  // added in this change (${seed})`, `  const extra${i} = value${i - 1} ?? 0`]
      : text.startsWith('name:')
        ? [`      - name: Added step (${seed})`, `        run: bun run extra${i}`]
        : [`Added in this change (${seed}): paragraph ${i} covers the new step.`]
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]!
    // Edits far enough apart that the unchanged stretches between them would fold, were folding on.
    if (i % 41 === 5) {
      out.push(/\+ \d+$/.test(line) ? line.replace(/\+ \d+$/, `+ ${seed * 10}`) : `${line} (revised)`)
      continue
    }
    if (i % 61 === 17) {
      continue
    }
    out.push(line)
    if (i % 53 === 9) {
      out.push(...inserted(i))
    }
  }
  return out.join('\n')
}

/**
 * What the conversation changed: the cookie rename with its test and helpers,
 * then the wider work around it across the monorepo, so the change view has
 * many files, deep directories and diffs with several hunks.
 */
const cookieBefore = cookieTs
  .replace("/** The session cookie: renamed from `sid` so the old name stops working. */\nexport const SESSION_COOKIE = 'session'", "export const SESSION_COOKIE = 'sid'")
  .replace("    sameSite: 'lax',\n", '')

const sidTs = `/** The old cookie name, kept for callers that still import it. */
export const SID = 'sid'
`

const authTestBefore = authTest
  .replace("expect(readSession('session=abc; theme=dark')).toBe('abc')", "expect(readSession('sid=abc; theme=dark')).toBe('abc')")
  .replace(`
  test('reads nothing without a header', () => {
    expect(readSession(undefined)).toBeNull()
  })
`, '')

const readmeBefore = readme.replace('bun run dev\n', 'bun run dev\nbun run typecheck\n')

interface ChangedSides {
  kind: ChangeFile['kind']
  from?: string
  original: string
  modified: string
}

function generatedChange(path: string, lines: number, seed: number): ChangedSides {
  const original = generated(path, lines, seed)
  return { kind: 'modified', original, modified: edited(original, seed) }
}

const changeSides: Record<string, ChangedSides> = {
  'src/auth/cookie.ts': { kind: 'modified', original: cookieBefore, modified: cookieTs },
  'src/auth/session.ts': { kind: 'added', original: '', modified: sessionTs },
  'src/auth/sid.ts': { kind: 'deleted', original: sidTs, modified: '' },
  'src/http/middleware.ts': generatedChange('src/http/middleware.ts', 72, 3),
  'tests/login/auth.test.ts': { kind: 'modified', original: authTestBefore, modified: authTest },
  'tests/login/session.test.ts': { kind: 'added', original: '', modified: generated('tests/login/session.test.ts', 30, 4) },
  'tests/http/router.test.ts': generatedChange('tests/http/router.test.ts', 48, 5),
  'README.md': { kind: 'modified', original: readmeBefore, modified: readme },
  'docs/guides/getting-started.md': {
    kind: 'renamed',
    from: 'docs/getting-started.md',
    original: '# getting-started\n\nInstall bun, then run the dev server.\n',
    modified: '# getting-started\n\nInstall bun, then run the dev server.\n\nThe session cookie is named `session`.\n',
  },
  'docs/demi-next/web-application.md': generatedChange('docs/demi-next/web-application.md', 40, 6),
  'packages/web-ui/src/agent/WorkPanel.vue': generatedChange('packages/web-ui/src/agent/WorkPanel.vue', 96, 7),
  'packages/web-ui/src/agent/work-panel.ts': generatedChange('packages/web-ui/src/agent/work-panel.ts', 60, 8),
  'packages/web-ui/src/agent/blocks/FileChangePills.vue': generatedChange('packages/web-ui/src/agent/blocks/FileChangePills.vue', 84, 9),
  'packages/web-ui/src/files/FileTree.vue': generatedChange('packages/web-ui/src/files/FileTree.vue', 120, 10),
  'packages/web-ui/src/files/changes.ts': { kind: 'added', original: '', modified: generated('packages/web-ui/src/files/changes.ts', 64, 11) },
  'packages/web/src/state/resources.ts': generatedChange('packages/web/src/state/resources.ts', 110, 12),
  'packages/agent/src/tools.ts': generatedChange('packages/agent/src/tools.ts', 90, 13),
  'scripts/release.ts': { kind: 'deleted', original: generated('scripts/release.ts', 36, 14), modified: '' },
  '.github/workflows/ci.yml': generatedChange('.github/workflows/ci.yml', 24, 15),
  'package.json': { kind: 'modified', original: packageJson.replace('"typecheck": "tsgo --noEmit"', '"typecheck": "tsc --noEmit"'), modified: packageJson },
  'assets/logo.svg': { kind: 'modified', original: logoSvgBefore, modified: logoSvg },
}

/**
 * Changed files that are not text: the committed version's asset, or that
 * it is over the 8 MiB git's copy is served up to; none for an added file.
 */
const binaryChanges: Record<string, { kind: ChangeFile['kind']; committed: { url: string; size: number } | 'too-large' | null }> = {
  'assets/photo.png': { kind: 'modified', committed: { url: '/fixtures/preview/photo-before.png', size: 16078 } },
  'assets/demo.mp4': { kind: 'added', committed: null },
  'assets/intro.mov': { kind: 'modified', committed: 'too-large' },
  'dist/app.zip': { kind: 'modified', committed: { url: '/fixtures/preview/app-before.zip', size: 124 } },
  'dist/cache.db': { kind: 'modified', committed: 'too-large' },
}

const changedFiles: ChangeFile[] = [
  ...Object.entries(changeSides).map(([path, sides]) => {
    const counts = lineCounts(sides.original, sides.modified)
    const change: ChangeFile = { path, kind: sides.kind, ...counts }
    if (sides.from) {
      change.from = sides.from
    }
    return change
  }),
  // Binary files count no lines, as the runner reports them.
  ...Object.entries(binaryChanges).map(([path, change]) => ({ path, kind: change.kind, added: 0, removed: 0 })),
]

/** The committed side of the binary changes, the way the raw committed route serves it. */
const committedContents: FileContents = {
  url: (path) => {
    const committed = binaryChanges[path]?.committed
    return committed && committed !== 'too-large' ? committed.url : ''
  },
  async describe(path) {
    const committed = binaryChanges[path]?.committed
    if (committed === 'too-large')
      throw new FileBrowserError('too-large', 'The file is over 8 MiB')
    if (!committed)
      throw new FileBrowserError('not-found', `The last commit has no ${path}`)
    return { size: committed.size, modifiedAt: null, version: null }
  },
}

/** How a fixture change set presents its listing, beyond the files themselves. */
export interface GalleryChangeListing {
  /** The list was cut short at the host's limit. */
  truncated?: boolean
  /** The last listing failed with these words; the files are the last good list. */
  failure?: string
  /** The workspace is not a git repository: no files at all. */
  unavailable?: 'no-repository'
}

/**
 * A change set the way the product's working tree presents one: reactive,
 * with a Refresh that shows as in flight for a moment, and the listing state
 * the specimen asks for.
 */
export function createGalleryChangeSet(latencyMs: number, listing: GalleryChangeListing = {}): ChangeSetSource {
  const fixed = createGalleryChanges(latencyMs)
  const source: ChangeSetSource = reactive({
    files: listing.unavailable ? [] : fixed.files,
    truncated: listing.truncated ?? false,
    unavailable: listing.unavailable ?? null,
    refreshing: false,
    failure: listing.failure ?? null,
    refresh() {
      source.refreshing = true
      setTimeout(() => {
        source.refreshing = false
      }, latencyMs * 4)
    },
    read: fixed.read,
    committed: committedContents,
  })
  return source
}

function createGalleryChanges(latencyMs: number): ChangeSetSource {
  return {
    files: changedFiles,
    async read(path, signal) {
      await new Promise<void>((resolve, reject) => {
        const timer = setTimeout(resolve, latencyMs)
        signal?.addEventListener('abort', () => {
          clearTimeout(timer)
          reject(new DOMException('Aborted', 'AbortError'))
        }, { once: true })
      })
      if (binaryChanges[path]) {
        throw new FileBrowserError('binary', 'The file is not UTF-8 text')
      }
      const sides = changeSides[path]
      if (!sides) {
        throw new Error(`No change recorded for ${path}`)
      }
      return { original: sides.original, modified: sides.modified }
    },
  }
}

export function createGalleryWorkspace(latencyMs = 200): {
  source: FileBrowserSource & { contents: FileContents }
  root: string
  changes: ChangeSetSource
} {
  const root = tree()
  // The workspace sits under the home directory the laptop fixtures use.
  const home = dir({ Projects: dir({ demi: root }) })
  const source = createMemoryFileSource({
    platform: 'macos',
    home: '/Users/zan',
    root: dir({ Users: dir({ zan: home }) }),
    latencyMs,
  })
  const changes = createGalleryChangeSet(latencyMs)
  return { source, root: WORKSPACE_ROOT, changes }
}
