import { createMemoryFileSource, dir, textFile, type MemoryDirectory } from '@demicodes/web-ui/files/memory-source'
import type { FileBrowserSource } from '@demicodes/web-ui/files/types'

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

const readme = `# demi

Conversations that run commands on Cloud or your own machines.

\`\`\`bash
bun install
bun run dev
\`\`\`
`

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
import type { ShellFileChange } from '../block-helpers'

defineProps<{ files: ShellFileChange[] }>()
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
          theme: dir(files('packages/web-ui/src/theme', ['appTheme.ts', 'codeThemes.ts', 'themeStore.ts'])),
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

export function createGalleryWorkspace(latencyMs = 200): { source: FileBrowserSource; root: string } {
  const root = tree()
  // The workspace sits under the home directory the laptop fixtures use.
  const home = dir({ Projects: dir({ demi: root }) })
  const source = createMemoryFileSource({
    platform: 'macos',
    home: '/Users/zan',
    root: dir({ Users: dir({ zan: home }) }),
    latencyMs,
  })
  return { source, root: WORKSPACE_ROOT }
}
