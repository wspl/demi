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

function tree(): MemoryDirectory {
  return dir({
    '.git': dir({ HEAD: textFile('ref: refs/heads/main\n', at) }),
    '.gitignore': textFile('node_modules\ndist\n', at),
    'package.json': textFile(packageJson, at),
    'README.md': textFile(readme, at),
    src: dir({
      auth: dir({
        'cookie.ts': textFile(cookieTs, at),
        'session.ts': textFile(sessionTs, at),
      }),
      'index.ts': textFile("export * from './auth/session'\n", at),
    }),
    tests: dir({
      login: dir({
        'auth.test.ts': textFile(authTest, at),
      }),
    }),
    packages: dir({
      'web-ui': dir({
        src: dir({
          agent: dir({
            blocks: dir({
              'FileChangePills.vue': textFile(pillsVue, at),
            }),
          }),
        }),
      }),
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
