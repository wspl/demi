export const previewMarkdown = `# Login test

The session cookie was renamed from \`sid\` to \`session\`. The helper already writes the new header. The login test still expects the old name.

## What to change

- [x] Read the failing expect in \`auth.test.ts\`
- [ ] Update the name assertion to \`session\`
- [ ] Leave \`cookie.ts\` alone

> Keep the fix in one file.

| Check | File | Result |
| --- | --- | --- |
| Helper | cookie.ts | writes \`session=\` |
| Test | auth.test.ts | still expects \`sid\` |
| Snapshot | auth.test.ts | stale header string |

Inline \`readSessionCookie\` and the expect:

\`\`\`ts
expect(readSessionCookie(header)).toEqual({
  name: 'session',
  value: 'abc',
})
\`\`\`

Open [auth.test.ts](packages/web/src/auth.test.ts) and fix the assertion.
`

export const previewCode = `import { describe, expect, test } from 'bun:test'
import { readSessionCookie } from './cookie'

describe('login session', () => {
  test('reads the session cookie after rename', () => {
    const header = 'session=abc; Path=/; HttpOnly'

    expect(readSessionCookie(header)).toEqual({
      name: 'session',
      value: 'abc',
    })
  })

  test('ignores an unrelated cookie', () => {
    const header = 'other=1; Path=/'

    expect(readSessionCookie(header)).toBeNull()
  })
})
`