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

/**
 * A reply with prices and math side by side: dollar signs before amounts stay
 * text, and math renders on its own and flush against Chinese text.
 */
export const dollarMarkdown = `Notion is $10 vs $12 a month; Slack $7.25 vs $8.75, and Zoom $13.33/$15.99.

The energy is $E = mc^2$, and 面积为$x^2$平方米.
`

/**
 * A reply whose lines outrun a narrow column: an address, a path, a long line of
 * code beside a short one, a wide table and a wide equation.
 */
export const overlongMarkdown = `Exposed 127.0.0.1:64921 on ZandeMacBook-Pro.local as http://c3t25fk3dbh3a67e5xyovsnhle.expose.localhost. Expires in 60 minutes (expose c3t25fk3dbh3a67e5xyovsnhle).

The test lives at \`packages/web-ui/src/files/__tests__/crumb-fit.test.ts\`.

\`\`\`
demi skills list
\`\`\`

\`\`\`
demi: this conversation needs the user's permission to manage skills; the request was sent to the user and this command failed.
\`\`\`

| Host | Address | Expires |
| --- | --- | --- |
| ZandeMacBook-Pro.local | http://c3t25fk3dbh3a67e5xyovsnhle.expose.localhost | in 60 minutes |

$$
\\int_0^1 x^2\\,dx + \\int_0^1 x^3\\,dx + \\int_0^1 x^4\\,dx + \\int_0^1 x^5\\,dx = \\frac{1}{3} + \\frac{1}{4} + \\frac{1}{5} + \\frac{1}{6}
$$
`
