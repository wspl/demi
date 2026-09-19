export const previewMarkdown = `# Login test

The session cookie was renamed from \`sid\` to \`session\`. The helper already writes the new header. The login test still expects the old name. Cookie names follow [RFC 6265](https://httpwg.org/specs/rfc6265.html).

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
