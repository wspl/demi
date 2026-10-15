import type { AgentMessage, Block, ChangedFile, CommandReport, EditedFile, ModelSelection, ToolResultContentBlock, UserContentBlock } from '@demicodes/protocol'
import { encodeRemoteReference } from '@demicodes/web-ui/agent/message-input/attachments'
import type { ShellToolView as ShellView, ToolCallBlock } from '@demicodes/web-ui/agent/block-types'
import type { ToolPicture } from '@demicodes/web-ui/agent/tool-media'
import { editCopies, galleryBlobSize, galleryBlobs, missingBlob } from './blobs'
import { agentReceiptMessages } from './subagents'

export type { ShellView }

/**
 * A complete shell view, the shape `shell` writes and every reader
 * validates. A specimen names only the parts it is about; the rest is what a
 * finished command leaves behind.
 */
export function shellView(
  parts: Partial<ShellView> & Pick<ShellView, 'chunks'>
): ShellView {
  const view: ShellView = {
    kind: 'shell',
    status: 'exited',
    commandId: 'cmd-demo',
    runningMs: 1_200,
    idleMs: 0,
    viewTruncated: false,
    ...parts,
  }
  // A command reports its exit code only once it has exited.
  if (view.status === 'exited')
    view.exitCode = parts.exitCode ?? 0
  return view
}

/**
 * A file a fixture command edited in `segments` segments, each segment's
 * sides held by the gallery's blobs: the cookie renamed once more per
 * segment, from the empty file when the command created it.
 */
export function editedFile(file: Omit<EditedFile, 'edits'>, segments = 1): EditedFile {
  const version = (n: number) => `// ${file.path}\nconst cookie = '${n === 0 ? 'sid' : n === 1 ? 'session' : `session-v${n}`}'\n`
  const edits = Array.from({ length: segments }, (_, n) => ({
    copies: editCopies(file.kind === 'added' && n === 0 ? '' : version(n), version(n + 1)),
  }))
  return { ...file, edits }
}

/** A file a fixture command edited without copies, such as a binary one. */
export function uncopiedFile(file: Omit<EditedFile, 'edits'>): EditedFile {
  return { ...file, edits: [{}] }
}

/**
 * `blocks` as one run of steps, as the backend stores it (`runtime.md`
 * § Work groups): each carries the id of the run's first block, which the
 * fixture names by putting the run's blocks together.
 */
export function inRun(...blocks: Block[]): Block[] {
  const run = blocks[0]!.id
  return blocks.map((block) => ({ ...block, run }) as Block)
}

/**
 * The entry of a run's or a request's list for `file`, which the call of
 * `commandId` alone changed, as the backend stores it: its lines those of
 * the call, none when a side of it was not kept.
 */
export function listedFile(commandId: string, file: EditedFile): ChangedFile {
  const kept = file.edits.length > 0 && file.edits.every((edit) => edit.copies)
  return {
    path: file.path,
    kind: file.kind,
    ...(file.from ? { from: file.from } : {}),
    ...(kept ? { lines: { added: file.added, removed: file.removed } } : {}),
    edits: file.edits.map((edit, segment) => ({
      call: commandId,
      path: file.path,
      segment,
      ...(file.kind === 'added' && segment === 0 ? { created: true } : {}),
      ...(edit.copies ? { copies: edit.copies } : {}),
    })),
  }
}

/** The files `block`, a shell call, changed, but outside its agent's directory, as entries of its lists. */
export function listedFiles(block: Block): ChangedFile[] {
  const view = block.type === 'tool_call' && block.view?.kind === 'shell' ? block.view : null
  return (view?.files ?? []).filter((file) => !file.outside).map((file) => listedFile(view!.commandId, file))
}

/**
 * `block`, a shell call, with its run's and its request's files as the
 * backend stored them; the request's are the run's unless named.
 */
export function withLists<T extends Block>(block: T, run: ChangedFile[], request: ChangedFile[] = run): T {
  if (block.type !== 'tool_call' || block.view?.kind !== 'shell')
    return block
  return { ...block, view: { ...block.view, runFiles: run, requestFiles: request } }
}

/** A shell call alone in its run and its request, with its files as both lists. */
export function ownLists<T extends Block>(block: T): T {
  return withLists(block, listedFiles(block))
}

export const demoModel: ModelSelection = {
  providerId: 'anthropic',
  model: {
    id: 'demo-model',
    name: 'Demo model',
    contextWindow: 200_000,
    outputLimit: null,
    thinking: [],
    acceptedExtensions: [],
  },
  thinking: null,
  serviceTierId: null,
}

/** The user's decisions on permission requests, as the agent that asked received them. */
export const permissionReceiptMessages: AgentMessage[] = [
  { outcome: 'allowed' as const, text: 'The user allowed this conversation to manage skills; the command `demi skills add vercel-labs/agent-skills --skill web-design-guidelines` can now run.' },
  { outcome: 'denied' as const, text: 'The user denied this conversation permission to manage skills; the command `demi skills remove acme/tools` was not run.' },
].map(({ outcome, text }, index) => ({
  id: `permission:pr-${index}`,
  recipientId: 'gallery-parent',
  timestamp: '2026-09-12T12:05:00.000Z',
  content: text,
  event: { type: 'permission' as const, outcome, action: 'manage skills' },
}))

/** The user's move of the conversation, which the user told the agent of. */
export const movedReceiptMessage: AgentMessage = {
  id: 'moved:4',
  recipientId: 'gallery-parent',
  timestamp: '2026-09-12T12:08:00.000Z',
  content: 'The user moved this conversation from Cloud (/home/demi/sessions/0f4e2a1c) to MacBook Pro (/Users/alex/code/photo-album). Files did not move. Check what this means for the work so far, and tell the user.',
  event: { type: 'moved', host: 'MacBook Pro', path: '/Users/alex/code/photo-album', home: '/Users/alex' },
}
/** A decision on a request of a category and a host, and Demi's notice that a move the agent asked for failed. */
export const organizeReceiptMessages: AgentMessage[] = [
  {
    id: 'permission:pr-move',
    recipientId: 'gallery-parent',
    timestamp: '2026-09-12T12:06:00.000Z',
    content:
      'The user allowed this conversation to organize conversations and reach build-box; the command `demi conversation move photo-album` can now run.',
    event: {
      type: 'permission' as const,
      outcome: 'allowed' as const,
      action: 'organize conversations and reach build-box',
      categories: ['conversation.organize'],
      host: 'build-box',
    },
  },
  {
    id: 'move-failed:4c1e9d2b',
    recipientId: 'gallery-parent',
    timestamp: '2026-09-12T12:08:00.000Z',
    content:
      "Demi could not make the move this conversation's agent asked for: the project no longer exists. The conversation still runs on Cloud (/home/demi/sessions/0f4e7597-e889-4baa-ac98-93fa61b68d28).",
    event: { type: 'move_failed' as const, reason: 'the project no longer exists' },
  },
]

export const demoImageUrl = '/fixtures/preview/photo.png'

export const longUserText = [
  'The login test in packages/web/src/auth.test.ts started failing after we renamed the session cookie from sid to session. CI is red on main and on this branch.',
  'The helper still writes Set-Cookie correctly. The assertion is what drifted: it looks for a sid= prefix and a Session header that we no longer send.',
  'I pasted the failing log and a screenshot from the last GitHub run. The request is a POST to /login with an email and password; the response is 204 and a session cookie.',
  'Please keep the fix inside auth.test.ts. Do not rename the helper, do not touch cookie.ts, and do not add a second test file just to isolate the assertion.',
  'The expired-cookie case can wait for a follow-up. I only want the rename covered so we can merge the cookie change today.',
  'If you need more context, the old name leaked into two comments and one snapshot string. Comments can stay; the snapshot has to match the new header.',
  'I already tried updating the snapshot locally. bun test packages/web/src/auth.test.ts still fails on the name field, so the expect() is the one that is wrong.',
  'When you are done, leave the rest of the suite alone. A green login test is enough for this turn.',
  'If the file is longer than you expect, scroll — the important expect is near the bottom, after the helper setup and the fixture header.',
].join('\n\n')

function iso(offsetMs: number): string {
  return new Date(Date.now() - offsetMs).toISOString()
}

/**
 * When a call that started at `createdAt` returned, as its block records
 * it: none while it executes; a shell call once its command ran the time its
 * view says, any other call at once.
 */
export function returnedAt(createdAt: string, status: ToolCallBlock['status'], view: ToolCallBlock['view']): string | null {
  if (status === 'executing')
    return null
  return new Date(Date.parse(createdAt) + (view?.kind === 'shell' ? view.runningMs : 0)).toISOString()
}

/** `call` with when it returned, as `returnedAt` says. */
export function withEnd(call: Omit<ToolCallBlock, 'endedAt'>): ToolCallBlock {
  return { ...call, endedAt: returnedAt(call.createdAt, call.status, call.view) }
}

function toolCall(
  partial: Pick<ToolCallBlock, 'id' | 'toolName' | 'input' | 'status'> & Partial<ToolCallBlock>
): ToolCallBlock {
  const createdAt = partial.createdAt ?? iso(120_000)
  return {
    type: 'tool_call',
    createdAt,
    model: demoModel,
    toolUseId: `${partial.id}-use`,
    output: [],
    view: null,
    endedAt: returnedAt(createdAt, partial.status, partial.view ?? null),
    ...partial,
  }
}

export const userPrompt: UserContentBlock[] = [
  {
    type: 'text',
    text: 'The login test in `packages/web/src/auth.test.ts` is failing after the session cookie rename. Keep the fix in that file.'
  },
]

export const steerPrompt: UserContentBlock[] = [
  {
    type: 'text',
    text: 'Do not touch the cookie helper. Only fix the assertion.'
  },
]

export const assistantMarkdown = `The cookie helper is fine. The test still expects \`sid\`.

I updated the assertion in [auth.test.ts](tests/login/auth.test.ts) and left [cookie.ts](src/auth/cookie.ts) alone. The login page the test drives, and the whole page as the test captured it:

![The login page](assets/photo.png)

![The whole login page](assets/page-full.png)

\`\`\`ts
expect(readSessionCookie(header)).toEqual({
  name: 'session',
  value: 'abc',
})
\`\`\`
`

export const thinkingText = `The cookie name changed from \`sid\` to \`session\`. The helper already writes the new header.

The test is the one still looking for \`sid\`, so the fix belongs in the test, not in the helper. 测试里仍在找旧的名字，所以应该改测试，而不是改 helper。

Let me search the test for the old name first.`

export const shellTool = toolCall({
  id: 'tool-shell',
  toolName: 'shell',
  status: 'completed',
  input: JSON.stringify({
    script: 'rg -n "sid" packages/web/src',
    description: 'Find the old cookie name',
  }),
  view: shellView({
    commandId: '11',
    // The Arabic match joins its letters in the output, as it does in a sentence.
    chunks: [
      {
        stream: 'stdout',
        text: [
          'packages/web/src/auth.test.ts:18:    expect(cookie.name).toBe("sid")',
          'packages/web/src/locales/ar.json:7:  "sidebar": "الشريط الجانبي",',
          '',
        ].join('\n'),
      },
    ],
  }),
})

/**
 * A call that returned while its command runs on, one of the conversation's
 * running commands (the gallery's `Run the auth tests` job): its row
 * shimmers until the command's end arrives.
 */
export const returnedShellTool = toolCall({
  id: 'tool-shell-returned',
  toolName: 'shell',
  status: 'completed',
  input: JSON.stringify({ script: 'bun test src/auth.test.ts', description: 'Run the auth tests' }),
  view: shellView({
    commandId: '12',
    status: 'running',
    chunks: [{ stream: 'stdout', text: 'bun test v1.3.14\n' }],
  }),
})

/**
 * Another call that returned while its command runs on; its command, which
 * the gallery runs beside the Running jobs, exits 1.
 */
export const returnedFailingShellTool = toolCall({
  id: 'tool-shell-returned-failing',
  toolName: 'shell',
  status: 'completed',
  input: JSON.stringify({ script: 'bun run typecheck --watch', description: 'Watch the type check' }),
  view: shellView({
    commandId: '13',
    status: 'running',
    chunks: [{ stream: 'stdout', text: 'Watching for changes…\n' }],
  }),
})

/**
 * The running call's script: long enough to wrap over more than two lines,
 * so the command shows two above the output, which scrolls under it.
 */
export const RUNNING_SHELL_SCRIPT = [
  'DEMI_LOG=auth=debug,cookie=debug,session=info bun test --watch --timeout 20000 --rerun-each 1 --bail 5 packages/web/src/auth.test.ts packages/web/src/cookie.test.ts packages/web/src/session.test.ts \\',
  '  2>&1 | tee target/auth-watch.log',
].join('\n')
export const RUNNING_SHELL_DESCRIPTION = 'Run the login test'

export const runningShellTool = toolCall({
  id: 'tool-shell-run',
  toolName: 'shell',
  status: 'executing',
  input: JSON.stringify({
    script: RUNNING_SHELL_SCRIPT,
    description: RUNNING_SHELL_DESCRIPTION,
  }),
  view: shellView({
    commandId: '17',
    status: 'running',
    chunks: [{ stream: 'stdout', text: 'bun test v1.2\n' }],
  }),
})

/**
 * A heredoc that writes a whole page, with output longer than the box: the
 * command shows two lines until a click shows it whole, and then scrolls on
 * its own above the output.
 */
export const longScriptShellTool = toolCall({
  id: 'tool-shell-script',
  toolName: 'shell',
  status: 'completed',
  input: JSON.stringify({
    script: [
      "cat > public/status.html <<'EOF'",
      '<!doctype html>',
      '<html lang="en">',
      '<head>',
      '  <meta charset="utf-8">',
      '  <title>Service Status</title>',
      '  <link rel="stylesheet" href="/styles/status.css">',
      '</head>',
      '<body>',
      '  <main>',
      '    <h1>Service Status</h1>',
      '    <p id="summary">All systems are running.</p>',
      '    <ul id="services"></ul>',
      '  </main>',
      '</body>',
      '</html>',
      'EOF',
      'grep -rn "sid" packages/web/src',
    ].join('\n'),
    description: 'Write the status page and list where the old cookie name is read',
  }),
  view: shellView({
    commandId: 'cmd-script',
    chunks: [
      {
        stream: 'stdout',
        text: Array.from(
          { length: 40 },
          (_, index) => `packages/web/src/${['auth', 'cookie', 'session', 'login'][index % 4]}.ts:${index * 3 + 7}:  const sid = cookies.get("sid")\n`,
        ).join(''),
      },
    ],
  }),
})

/**
 * A script of a few lines, as Bash highlights it: a variable with a default,
 * quotes, a pipe, a loop and a heredoc.
 */
export const BASH_SCRIPT = [
  'SERVICE="${1:-auth}"',
  'grep -rn "sid" "packages/web/src/$SERVICE" | wc -l',
  'for file in $(git diff --name-only); do echo "changed: $file"; done',
  "cat > /tmp/check.sh <<'EOF'",
  'bun test "$SERVICE" --bail',
  'EOF',
].join('\n')

export const bashScriptShellTool = toolCall({
  id: 'tool-shell-bash',
  toolName: 'shell',
  status: 'completed',
  input: JSON.stringify({ script: BASH_SCRIPT, description: 'Count the old cookie name and list the changed files' }),
  view: shellView({
    commandId: 'cmd-bash',
    chunks: [{ stream: 'stdout', text: '4\nchanged: packages/web/src/auth.test.ts\nchanged: packages/web/src/cookie.ts\n' }],
  }),
})

/**
 * One long line of words with hyphens inside them and a signed address
 * longer than any line: the command wraps at its spaces, each word whole,
 * and only the address breaks inside, filling its lines to the edge. The
 * output has the same: a sentence that wraps at its spaces, a table aligned
 * with tabs, and a hash longer than a line.
 */
export const longLineShellTool = toolCall({
  id: 'tool-shell-long-line',
  toolName: 'shell',
  status: 'completed',
  input: JSON.stringify({
    script: [
      'cd "$(mktemp -d)" && sudo apt-get install -y --no-install-recommends libatk-bridge2.0-0t64 libgtk-3-0t64 libxkbcommon-x11-0;',
      'echo "exit=$?"; curl -fsSL',
      'https://downloads.example.test/runner/demi-runner-aarch64-apple-darwin.tar.gz?expires=1791763200&signature=3q2-7wAAAAB1dGYtOC1lbmNvZGVkLXNpZ25hdHVyZS1mb3ItdGhlLWdhbGxlcnk',
      '| tar -xz && ./demi-runner --version',
    ].join(' '),
    description: 'Install the browser libraries and the runner',
  }),
  view: shellView({
    commandId: 'cmd-long-line',
    chunks: [
      {
        stream: 'stdout',
        text: [
          'Reading package lists... Done',
          'libatk-bridge2.0-0t64 is already the newest version (2.52.0-1build1).',
          'exit=0',
          'demi-runner 0.42.0',
          'Warning: the download location redirected to another host, so the runner archive was fetched from the new address; run the command again with the redirect option set to follow it to the end, and compare the checksum below.',
          'name\tstate\tsize',
          'demi-runner\tready\t14.2 MB',
          'sha256:3f2c9a1e7b5d4c8806fa12e9d3b7a45c0e8f1d26b9a374c5e0d18f6a2b7c93d4e1f05a68b2c7d93e4f1a0b8c5d62e7f93a1b4c8d05e6f2a7b19c3d8e4f60a5b7c2d1e9f38a40b6c5d7e2f1a9b83c40d6e5f7a2b1c98d3e04f6a5b7c',
          '',
        ].join('\n'),
      },
    ],
  }),
})

export const editingShellTool = toolCall({
  id: 'tool-shell-edit',
  toolName: 'shell',
  status: 'completed',
  input: JSON.stringify({
    script: 'sed -i "s/sid/session/" packages/web/src/auth.test.ts && bun test packages/web/src/auth.test.ts',
    description: 'Rename the cookie in the login test',
  }),
  view: shellView({
    commandId: 'cmd-edit',
    chunks: [{ stream: 'stdout', text: 'bun test v1.2\n 3 pass\n 0 fail\n' }],
    files: [
      editedFile({ path: 'packages/web/src/auth.test.ts', kind: 'modified', added: 12, removed: 3 }),
      editedFile({ path: 'packages/web/src/cookie.ts', kind: 'modified', added: 1, removed: 1 }),
    ],
  }),
})

/** The line a command's output shows for its binary stdout (`runtime.md` § What `demi file view` shows). */
function binaryLine(bytes: number): string {
  return `<binary stdout: ${bytes} bytes>\n`
}

/**
 * A shell call that viewed one image or video with `demi file view`
 * (`runtime.md` § Media the model views): its output holds the medium's line,
 * which the runner wrote, and the result attaches the medium, or the part
 * that took its place, after it.
 */
function viewedCall(
  partial: Pick<ToolCallBlock, 'id' | 'toolName' | 'input'>,
  commandId: string,
  line: string,
  medium: ToolResultContentBlock,
  files?: ShellView['files'],
): ToolCallBlock {
  return toolCall({
    ...partial,
    status: 'completed',
    output: [{ type: 'text', text: `status: exited\nexitCode: 0\ncommandId: ${commandId}\noutput:\n${line}\n` }, medium],
    view: shellView({
      commandId,
      chunks: [{ stream: 'stdout', text: `${line}\n` }],
      ...(files ? { files } : {}),
    }),
  })
}

/** An image's line in a job's output, as the runner writes it. */
function imageLine(number: number, width: number, height: number, bytes: number, mediaType = 'image/png'): string {
  return `[image ${number}: ${mediaType}, ${width} × ${height} px, ${bytes} bytes]`
}

function blobImage(ref: string): ToolResultContentBlock {
  return { type: 'image', source: { type: 'ref', ref, mediaType: 'image/png', ...galleryBlobSize(ref) } }
}

const screenshotInput = JSON.stringify({
  script: 'demi browser screenshot t1 | demi file view',
  description: 'Take a screenshot of the login page',
})

/**
 * One screenshot piped into `demi file view`: what it captured, which the
 * screenshot writes to stderr, its bytes, and the part the result holds for
 * it, its image or the part that says it is gone.
 */
interface Capture {
  tab: string
  width: number
  height: number
  bytes: number
  medium: ToolResultContentBlock
}

/** What a screenshot writes to stderr, as `demi browser screenshot` writes it. */
function captureText(capture: Capture): string {
  return `Screenshot of ${capture.tab}\nImage: ${capture.width} × ${capture.height} px, one pixel per CSS pixel\nViewport: ${capture.width} × ${capture.height} CSS px, device pixel ratio 2, web\n`
}

/**
 * A shell call whose screenshots it viewed (`runtime.md` § Media the model
 * views): the output holds each screenshot's description, from its stderr,
 * and the line `[image n: …]` the runner wrote, then the lines of the media
 * it viewed after them, `after`; the result attaches the media after the
 * output, in order, then the lines for those it did not attach.
 */
function screenshotsCall(
  partial: Pick<ToolCallBlock, 'id' | 'toolName' | 'input'> & Partial<Pick<ToolCallBlock, 'createdAt'>>,
  commandId: string,
  captures: Capture[],
  notes: string[] = [],
  after: string[] = [],
): ToolCallBlock {
  const chunks = [
    ...captures.flatMap((capture, index) => [
      { stream: 'stderr' as const, text: captureText(capture) },
      { stream: 'stdout' as const, text: `${imageLine(index + 1, capture.width, capture.height, capture.bytes)}\n` },
    ]),
    ...after.map((line) => ({ stream: 'stdout' as const, text: `${line}\n` })),
  ]
  const output = chunks.map((chunk) => chunk.text).join('')
  return toolCall({
    ...partial,
    status: 'completed',
    output: [
      { type: 'text', text: `status: exited\nexitCode: 0\ncommandId: ${commandId}\noutput:\n${output}` },
      ...captures.map((capture) => capture.medium),
      ...(notes.length > 0 ? [{ type: 'text' as const, text: notes.join('\n') }] : []),
    ],
    view: shellView({ commandId, chunks }),
  })
}

/** A screenshot the agent viewed, under its call (`file-previews.md` § Media a tool returned). */
export const screenshotTool = screenshotsCall(
  { id: 'tool-screenshot', toolName: 'shell', input: screenshotInput },
  '17',
  [{ tab: 't1', width: 480, height: 300, bytes: 15_822, medium: blobImage(galleryBlobs.screenshot) }],
)

/**
 * Three tabs in one call: a loop of screenshots viewed, each image attached
 * in the loop's order; a fourth, which the model does not read, is told of
 * after them.
 */
export const screenshotsTool = screenshotsCall(
  {
    id: 'tool-screenshots',
    toolName: 'shell',
    input: JSON.stringify({
      script: 'for t in t1 t2 t3; do demi browser screenshot "$t" | demi file view; done; demi file view mock.webp',
      description: 'Compare the login page in three tabs and the mock',
    }),
  },
  '18',
  [
    { tab: 't1', width: 480, height: 300, bytes: 15_822, medium: blobImage(galleryBlobs.screenshot) },
    { tab: 't2', width: 480, height: 300, bytes: 16_078, medium: blobImage(galleryBlobs.chart) },
    { tab: 't3', width: 360, height: 2400, bytes: 8_035, medium: blobImage(galleryBlobs.fullPage) },
  ],
  ['[image 4: not attached: the model does not read image/webp]'],
  [imageLine(4, 480, 300, 9_120, 'image/webp')],
)

/** `count` screenshots of the login page, alternating between the page and its chart, as a loop over tabs takes them. */
function captures(count: number): Capture[] {
  return Array.from({ length: count }, (_, index) => index % 2 === 0
    ? { tab: `t${index + 1}`, width: 480, height: 300, bytes: 15_822, medium: blobImage(galleryBlobs.screenshot) }
    : { tab: `t${index + 1}`, width: 480, height: 300, bytes: 16_078, medium: blobImage(galleryBlobs.chart) })
}

/**
 * `count` pictures as a row holds them in its line, alternating between the
 * login page and its chart (480 × 300, so each takes 32 × 20).
 */
export function rowPictureSamples(count: number): ToolPicture[] {
  return Array.from({ length: count }, (_, index): ToolPicture => {
    const medium = blobImage(index % 2 === 0 ? galleryBlobs.screenshot : galleryBlobs.chart)
    if (medium.type !== 'image')
      throw new Error('a blob image is an image')
    return { kind: 'image', source: medium.source, name: `Screenshot ${index + 1}` }
  })
}

/** Four screenshots in one call: they stand in the call's line, beside its title. */
export const fourScreenshotsTool = screenshotsCall(
  {
    id: 'tool-four-screenshots',
    toolName: 'shell',
    input: JSON.stringify({
      script: 'for t in t1 t2 t3 t4; do demi browser screenshot "$t" | demi file view; done',
      description: 'Take screenshots of the login page in four tabs',
    }),
  },
  'cmd-four-shots',
  captures(4),
)

/** Twelve screenshots in one call: as many as fit in the call's line, the rest counted as +N. */
export const twelveScreenshotsTool = screenshotsCall(
  {
    id: 'tool-twelve-screenshots',
    toolName: 'shell',
    input: JSON.stringify({
      script: 'for i in $(seq 12); do demi browser screenshot t1 | demi file view; demi browser scroll t1 --by 600; done',
      description: 'Take a screenshot of every screenful of the sign-in flow as it scrolls down the page',
    }),
  },
  'cmd-twelve-shots',
  captures(12),
)

/**
 * A run that opened a page and took twelve screenshots of it: folded, the
 * run's row holds its steps' pictures in its own line, as many as fit.
 */
export function screenshotRunBlocks(): Block[] {
  const open = toolCall({
    id: 'shots-run-open',
    createdAt: iso(590_000),
    toolName: 'shell',
    status: 'completed',
    input: JSON.stringify({ script: 'demi browser open http://localhost:5173/login', description: 'Open the sign-in page' }),
    output: [{ type: 'text', text: 'status: exited\nexitCode: 0\ncommandId: 71\noutput:\nt1\n' }],
    view: shellView({ commandId: '71', chunks: [{ stream: 'stdout', text: 't1\n' }] }),
  })
  const shots = { ...twelveScreenshotsTool, id: 'shots-run-shots', toolUseId: 'shots-run-shots', createdAt: iso(585_000) }
  return [
    {
      type: 'user', id: 'shots-run-ask', turnId: 'shots-run-turn', createdAt: iso(600_000), model: demoModel,
      content: [{ type: 'text', text: 'Screenshot the whole sign-in flow.' }], preamble: null,
    },
    ...inRun(open, shots),
    { type: 'text', id: 'shots-run-answer', createdAt: iso(580_000), model: demoModel, text: 'Twelve screenshots, one per screenful: the form, its errors, and the dashboard after it.' },
  ]
}

/**
 * A transcript with the pictures the media viewer steps through, in the
 * order they stand: the user's attached screenshot, three screenshots a
 * call viewed, and a reply's two images of the workspace and an attachment.
 */
export function viewerTranscriptBlocks(): Block[] {
  const shots = { ...screenshotsTool, id: 'viewer-shots', toolUseId: 'viewer-shots', createdAt: iso(585_000) }
  return [
    {
      type: 'user', id: 'viewer-ask', turnId: 'viewer-turn', createdAt: iso(600_000), model: demoModel,
      content: [
        { type: 'text', text: 'The sign-in page looks wrong, as in ' },
        { type: 'image', source: { type: 'ref', ref: galleryBlobs.screenshot, mediaType: 'image/png', width: 480, height: 300 } },
        {
          type: 'attachment',
          name: 'login-fail.png',
          path: '/home/demi/.demi/attachments/demo/login-fail.png',
          mediaType: 'image/png',
          sizeBytes: 48211,
          sha256: galleryBlobs.screenshot,
        },
        { type: 'text', text: '. Compare it with the other tabs.' },
      ],
      preamble: null,
    },
    ...inRun(shots),
    {
      type: 'text', id: 'viewer-answer', createdAt: iso(580_000), model: demoModel,
      text: 'The tabs differ only in the button. The test pattern and the whole page, from the workspace:\n\n![The test pattern](assets/photo.png)\n![The whole page](assets/page-full.png)\n\nAnd the timeline of the steps:\n\n![The timeline of the steps](attachment:a5)',
    },
  ]
}

/** A capture of a whole page: taller than a thumbnail's proportions, so the thumbnail keeps its top. */
export const fullPageTool = viewedCall(
  {
    id: 'tool-full-page',
    toolName: 'shell',
    input: JSON.stringify({
      script: 'demi browser screenshot t1 --full-page 2>/dev/null | convert - -strip png:- | demi file view',
      description: 'Capture the whole login page',
    }),
  },
  'cmd-full-page',
  imageLine(1, 360, 2400, 8_035),
  blobImage(galleryBlobs.fullPage),
)

/** A timeline wider than a thumbnail's proportions: the thumbnail keeps its middle. */
export const wideImageTool = viewedCall(
  {
    id: 'tool-wide-image',
    toolName: 'shell',
    input: JSON.stringify({
      script: 'demi file view out/timeline.png',
      description: 'Show the timeline of the run',
    }),
  },
  'cmd-timeline',
  imageLine(1, 1600, 300, 5_883),
  blobImage(galleryBlobs.timeline),
)

/** An icon: its row picture is scaled down to 20 × 20, and its card keeps its own 48 × 48, never enlarged. */
export const smallImageTool = viewedCall(
  {
    id: 'tool-small-image',
    toolName: 'shell',
    input: JSON.stringify({
      script: 'demi file view public/favicon.png',
      description: 'Show the favicon',
    }),
  },
  'cmd-favicon',
  imageLine(1, 48, 48, 972),
  blobImage(galleryBlobs.icon),
)

/**
 * Eight frames of a progress strip, each 171 × 40, wider than a row
 * picture's proportions: each shows its middle, 48 × 20, and those that do
 * not fit are counted.
 */
export const shortFramesTool = (() => {
  const frames = Array.from({ length: 8 }, (_, index) => index + 1)
  const lines = frames.map((n) => imageLine(n, 171, 40, 337)).join('\n')
  return toolCall({
    id: 'tool-short-frames',
    toolName: 'shell',
    input: JSON.stringify({
      script: 'for f in out/progress-*.png; do demi file view "$f"; done',
      description: 'Show the frames of the progress strip',
    }),
    status: 'completed',
    output: [
      { type: 'text', text: `status: exited\nexitCode: 0\ncommandId: 19\noutput:\n${lines}\n` },
      ...frames.map(() => blobImage(galleryBlobs.frame)),
    ],
    view: shellView({ commandId: '19', chunks: [{ stream: 'stdout', text: `${lines}\n` }] }),
  })
})()

/**
 * A recording the agent viewed: its first frame with a play mark, which a
 * click plays in the viewer.
 */
export const recordingTool = viewedCall(
  {
    id: 'tool-recording',
    toolName: 'shell',
    input: JSON.stringify({
      script: 'demi file view out/checkout.webm',
      description: 'Show the recording of the checkout flow',
    }),
  },
  'cmd-record',
  '[video 1: video/webm, 3.0 s, 320 × 180 px, 23336 bytes]',
  {
    type: 'video',
    source: { type: 'ref', ref: galleryBlobs.recording, mediaType: 'video/webm', ...galleryBlobSize(galleryBlobs.recording) },
  },
)

/** A recording whose reference carries no size: its thumbnail is 16:9 until its first frame arrives. */
export const unsizedRecordingTool = viewedCall(
  {
    id: 'tool-recording-unsized',
    toolName: 'shell',
    input: JSON.stringify({
      script: 'demi file view out/demo.mp4',
      description: 'Show the demo of the checkout',
    }),
  },
  'cmd-demo',
  '[video 1: video/mp4, 31822 bytes]',
  { type: 'video', source: { type: 'ref', ref: galleryBlobs.unsizedRecording, mediaType: 'video/mp4' } },
)

/**
 * A command that exited after its call returned: the look that saw the end,
 * a `shell` call that runs `demi shell status`, carries the picture it viewed.
 */
export const statusImageTool = toolCall({
  id: 'tool-status-image',
  toolName: 'shell',
  status: 'completed',
  input: JSON.stringify({ script: 'demi shell status 24', description: 'Check the latency chart', intervalMs: 15_000 }),
  output: [
    { type: 'text', text: `status: exited\nexitCode: 0\ncommandId: 25\noutput:\nstatus: exited\nexitCode: 0\ncommandId: 24\noutput:\n${imageLine(1, 480, 300, 16_078)}\n` },
    blobImage(galleryBlobs.chart),
  ],
  view: shellView({
    commandId: '25',
    chunks: [{ stream: 'stdout', text: `status: exited\nexitCode: 0\ncommandId: 24\noutput:\n${imageLine(1, 480, 300, 16_078)}\n` }],
  }),
})

/** A recording whose bytes could not be stored: in its place, the part that says so and why. */
export const notStoredVideoTool = viewedCall(
  {
    id: 'tool-recording-not-stored',
    toolName: 'shell',
    input: JSON.stringify({
      script: 'demi file view out/checkout.webm',
      description: 'Show the recording of the checkout flow',
    }),
  },
  'cmd-record-lost',
  '[video 1: video/webm, 3.0 s, 320 × 180 px, 23336 bytes]',
  {
    type: 'gone',
    kind: 'video',
    mediaType: 'video/webm',
    cause: { type: 'not_stored', error: 'the object store refused the write' },
  },
)

/** A screenshot whose blob the page cannot load. */
export const missingImageTool = screenshotsCall(
  { id: 'tool-screenshot-missing', toolName: 'shell', input: screenshotInput },
  '11',
  [{ tab: 't1', width: 480, height: 300, bytes: 15_822, medium: blobImage(missingBlob) }],
)

/**
 * An image printed rather than viewed: its stdout is binary, which the
 * output shows as one line and the result never attaches, saying instead
 * how to look at it.
 */
export const binaryStdoutTool = toolCall({
  id: 'tool-binary-stdout',
  toolName: 'shell',
  status: 'completed',
  input: JSON.stringify({ script: 'cat out/timeline.png', description: 'Print the timeline of the run' }),
  output: [
    { type: 'text', text: `status: exited\nexitCode: 0\ncommandId: cmd-printed\noutput:\n${binaryLine(5_883)}` },
    {
      type: 'text',
      text: '[binary stdout, 5883 bytes: not shown; to look at an image, a video or a PDF, pipe it into demi file view; to keep it, redirect it to a file]',
    },
  ],
  view: shellView({ commandId: 'cmd-printed', chunks: [{ stream: 'stdout', text: binaryLine(5_883) }] }),
})

/**
 * Pills of one name and one count are equally wide; this many fill the two
 * lines of the 360px (phone) specimen exactly, so all show and none is
 * counted.
 */
const EXACT_FIT = 6

/** A call that removed screenshots, rewrote some pages and added a few files, seventy in all. */
function seventyFiles(id: string): ToolCallBlock {
  return fileChangeCase(id, 'Clean up the screenshots', Array.from({ length: 70 }, (_, i) => {
    if (i % 7 === 0)
      return editedFile({ path: `packages/web/src/pages/Page${i}.vue`, kind: 'added', added: 12 + i, removed: 0 })
    if (i % 7 === 1)
      return editedFile({ path: `packages/web/src/pages/Page${i}.vue`, kind: 'modified', added: i, removed: i % 5 + 1 })
    return uncopiedFile({ path: `out/screenshots/login-${i}.png`, kind: 'deleted', added: 0, removed: 0 })
  }))
}

function fileChangeCase(
  id: string,
  description: string,
  files: ShellView['files'],
  status: ToolCallBlock['status'] = 'completed',
): ToolCallBlock {
  return toolCall({
    id: `tool-files-${id}`,
    toolName: 'shell',
    status,
    input: JSON.stringify({ script: `demi-edit ${id}`, description }),
    view: shellView({
      commandId: `cmd-files-${id}`,
      chunks: [{ stream: 'stdout', text: 'done\n' }],
      files,
    }),
  })
}

/** One shell call per way a command can touch files, from the common single edit to the overflow. */
export const fileChangeCases: { variant: string, width?: number, block: ToolCallBlock }[] = [
  {
    variant: 'one edit',
    block: fileChangeCase('one', 'Fix the cookie assertion', [
      editedFile({ path: 'packages/web/src/auth.test.ts', kind: 'modified', added: 1, removed: 1 }),
    ]),
  },
  {
    variant: 'new file',
    block: fileChangeCase('new', 'Add the session helper', [
      editedFile({ path: 'packages/web/src/session.ts', kind: 'added', added: 40, removed: 0 }),
    ]),
  },
  {
    variant: 'append only, not new',
    block: fileChangeCase('append', 'Note the rename in the readme', [
      editedFile({ path: 'packages/web/README.md', kind: 'modified', added: 3, removed: 0 }),
    ]),
  },
  {
    variant: 'contents unavailable',
    block: fileChangeCase('unavailable', 'Update the binary asset', [
      uncopiedFile({ path: 'assets/logo.png', kind: 'modified', added: 0, removed: 0 }),
    ]),
  },
  {
    variant: 'two edits to one file',
    block: fileChangeCase('segments', 'Update the cookie across interleaved writes', [
      editedFile({ path: 'packages/web/src/auth.test.ts', kind: 'modified', added: 2, removed: 2 }, 2),
    ]),
  },
  {
    variant: 'a few, mixed',
    block: fileChangeCase('mixed', 'Move the cookie name into one module', [
      editedFile({ path: 'packages/web/src/auth.test.ts', kind: 'modified', added: 12, removed: 3 }),
      editedFile({ path: 'packages/web/src/session.ts', kind: 'added', added: 40, removed: 0 }),
      editedFile({ path: 'packages/web/package.json', kind: 'modified', added: 1, removed: 1 }),
    ]),
  },
  {
    variant: 'long names',
    block: fileChangeCase('long', 'Regenerate the snapshots', [
      editedFile({ path: 'packages/web/src/__snapshots__/auth.test.ts.snap', kind: 'modified', added: 2, removed: 2 }),
      editedFile({ path: 'packages/web/src/components/ConversationListDropdownItemWithAVeryLongName.vue', kind: 'modified', added: 5, removed: 5 }),
      editedFile({ path: 'packages/web/src/components/ConversationListDropdownItemWithAVeryLongName.test.ts', kind: 'added', added: 120, removed: 0 }),
    ]),
  },
  {
    variant: 'forty files, two lines and the rest counted',
    block: fileChangeCase('many', 'Rename the prop across every widget', Array.from({ length: 40 }, (_, i) => editedFile({
      path: `packages/web/src/components/Widget${i + 1}.vue`,
      kind: i % 9 === 0 ? 'added' : 'modified',
      added: i + 1,
      removed: i % 3,
    }))),
  },
  {
    variant: 'seventy files, added, modified and removed',
    block: seventyFiles('seventy'),
  },
  {
    variant: 'seventy files · phone width',
    width: 360,
    block: seventyFiles('seventy-phone'),
  },
  {
    variant: 'fits in two lines exactly · phone width',
    width: 360,
    block: fileChangeCase('exact', 'Rename the cookie in the auth modules', Array.from({ length: EXACT_FIT }, (_, i) => editedFile({
      path: `${String.fromCharCode(97 + i)}/auth.ts`,
      kind: 'modified',
      added: 1,
      removed: 1,
    }))),
  },
  {
    variant: 'still running',
    block: fileChangeCase('running', 'Apply the codemod', [], 'executing'),
  },
  {
    variant: 'no files',
    block: fileChangeCase('none', 'List the test files', []),
  },
]

const caseBlock = (variant: string): Block =>
  fileChangeCases.find((item) => item.variant === variant)!.block as Block

/**
 * A heavy refactor turn: many shell calls in a row, most touching one or two
 * files, one sweeping forty, one still running. The Changes view shows it
 * whole so the pills are judged in a real transcript, not one row at a time.
 */
export function changesDemoBlocks(): Block[] {
  const text = (id: string, offsetMs: number, body: string): Block => ({
    type: 'text',
    id,
    createdAt: iso(offsetMs),
    model: demoModel,
    text: body,
  })
  const snapshotDiff = viewedCall(
    {
      id: 'changes-snapshot-diff',
      toolName: 'shell',
      input: JSON.stringify({
        script: 'bun scripts/snapshot-diff.ts login && demi file view out/login-diff.png',
        description: 'Compare the login snapshot before and after',
      }),
    },
    'cmd-snapshot-diff',
    imageLine(1, 480, 300, 16_078),
    blobImage(galleryBlobs.chart),
    [
      uncopiedFile({ path: 'out/login-diff.png', kind: 'added', added: 0, removed: 0 }),
      editedFile({ path: 'scripts/snapshot-diff.ts', kind: 'modified', added: 4, removed: 1 }),
    ],
  )
  // The lists the latest call of each run stored: each file once, with its
  // lines from before the run, or the request, to after the call.
  const listed = (variant: string) => listedFiles(caseBlock(variant))
  const [loginTest] = listed('one edit')
  const [twice] = listed('two edits to one file')
  const [session] = listed('new file')
  const loginInRun = { ...loginTest!, lines: { added: 3, removed: 3 }, edits: [...loginTest!.edits, ...twice!.edits] }
  const firstRun = [loginInRun, session!, ...listed('contents unavailable')]
  const [mixedTest, mixedSession, ...mixedRest] = listed('a few, mixed')
  const secondRun = [...listed('forty files, two lines and the rest counted'), mixedTest!, mixedSession!, ...mixedRest, ...listed('long names'), ...listed('append only, not new')]
  const thirdRun = listedFiles(snapshotDiff)
  // The request lists the login test and the session helper once, where
  // its first run put them, with the second run's edits of them.
  const request = [
    { ...loginInRun, lines: { added: 15, removed: 6 }, edits: [...loginInRun.edits, ...mixedTest!.edits] },
    { ...session!, edits: [...session!.edits, ...mixedSession!.edits] },
    ...firstRun.slice(2),
    ...secondRun.filter((file) => file !== mixedTest && file !== mixedSession),
  ]
  return [
    {
      type: 'user',
      id: 'changes-user',
      turnId: 'changes-turn',
      createdAt: iso(200_000),
      model: demoModel,
      content: [{
        type: 'text',
        text: 'Rename the `sid` cookie to `session` across packages/web. Keep every widget compiling and regenerate the snapshots.',
      }],
      preamble: null,
    },
    ...inRun(
      {
        type: 'thinking',
        id: 'changes-thinking',
        createdAt: iso(190_000),
        model: demoModel,
        text: 'The name lives in cookie.ts, the login test, the snapshots and a prop on every widget. Start from the helper, then sweep the widgets with a codemod.',
        signature: null,
      },
      shellTool as Block,
      caseBlock('one edit'),
      caseBlock('new file'),
      caseBlock('contents unavailable'),
      withLists(caseBlock('two edits to one file'), firstRun, firstRun),
    ),
    text('changes-text-1', 150_000, 'The helper now writes `session`. Sweeping the widgets next; each one reads the cookie name from a prop.'),
    ...inRun(
      caseBlock('forty files, two lines and the rest counted'),
      caseBlock('a few, mixed'),
      caseBlock('no files'),
      caseBlock('long names'),
      withLists(caseBlock('append only, not new'), secondRun, request),
    ),
    text('changes-text-2', 60_000, 'Every widget compiles and the snapshots match. Applying the same codemod to the docs examples.'),
    ...inRun(withLists(snapshotDiff, thirdRun, [...request, ...thirdRun]), caseBlock('still running')),
  ]
}

/**
 * What `demi shell status <commandId>` prints of a command, as a look's own
 * command shows it in its output.
 */
function statusText(commandId: string, status: 'running' | 'exited', output: string): string {
  return status === 'running'
    ? `status: running\ncommandId: ${commandId}\noutput:\n${output}`
    : `status: exited\nexitCode: 0\ncommandId: ${commandId}\noutput:\n${output}`
}

/**
 * A `shell` call whose script only looks at a command with `demi shell
 * status`: a call of its own, titled by its description, whose command
 * printed what it saw.
 */
function lookCall(
  id: string,
  description: string,
  looked: string,
  ownCommandId: string,
  seen: string,
  at = 120_000,
): ToolCallBlock {
  return toolCall({
    id,
    createdAt: iso(at),
    toolName: 'shell',
    status: 'completed',
    input: JSON.stringify({ script: `demi shell status ${looked}`, description, intervalMs: 15_000 }),
    view: shellView({ commandId: ownCommandId, chunks: [{ stream: 'stdout', text: seen }] }),
  })
}

/** A look at the login test: the script only runs `demi shell status 17`. */
export const lookTool = lookCall(
  'tool-look',
  'Check the login test',
  '17',
  '20',
  statusText('17', 'running', '✓ auth.test.ts > signs in (12 ms)\n'),
)

/** An answer to the prompt the login test waits for, then a look at what it did with it. */
export const inputTool = toolCall({
  id: 'tool-input',
  toolName: 'shell',
  status: 'completed',
  input: JSON.stringify({
    script: "demi shell input 17 <<'EOF'\nr\nEOF\ndemi shell status 17",
    description: 'Rerun the failed tests',
    intervalMs: 15_000,
  }),
  view: shellView({
    commandId: '21',
    chunks: [{ stream: 'stdout', text: statusText('17', 'running', 'Rerunning 1 failed test\n') }],
  }),
})

/** A stop of the login test, which a command of the shell does. */
export const stopTool = toolCall({
  id: 'tool-stop',
  toolName: 'shell',
  status: 'completed',
  input: JSON.stringify({
    script: 'demi shell stop 17',
    description: 'Stop the login test',
    intervalMs: 15_000,
  }),
  view: shellView({
    commandId: '22',
    chunks: [{ stream: 'stdout', text: '[command 17 stopped]\n' }],
  }),
})

/**
 * A command that failed: its result is an error to the model, and its row
 * says so with its tag, and in words above its output once opened.
 */
export const errorTool = toolCall({
  id: 'tool-error',
  toolName: 'shell',
  status: 'error',
  input: JSON.stringify({ script: 'rm /tmp/demi.lock', description: 'Remove the stale lock file', intervalMs: 15_000 }),
  output: [{ type: 'text', text: "status: exited\nexitCode: 1\ncommandId: 23\noutput:\nrm: cannot remove '/tmp/demi.lock': Permission denied\n" }],
  view: shellView({
    commandId: '23',
    exitCode: 1,
    chunks: [{ stream: 'stderr', text: "rm: cannot remove '/tmp/demi.lock': Permission denied\n" }],
  }),
})

/**
 * The calls the end specimens show: commands that succeeded, failed, were
 * stopped, were lost or still run, one with a title too long for a narrow
 * row, looks at them, and calls that ran nothing: the repeat guard's and a
 * refused input's.
 */
export function commandEndBlocks(): ToolCallBlock[] {
  type End =
    | { status: 'exited', exitCode: number }
    | { status: 'running' | 'aborted' }
    | { status: 'lost', lostReason: string }
  const failed = (end: End) => end.status === 'aborted' || end.status === 'lost' || (end.status === 'exited' && end.exitCode !== 0)
  const run = (id: string, commandId: string, description: string, end: End) => toolCall({
    id,
    toolName: 'shell',
    status: failed(end) ? 'error' : 'completed',
    input: JSON.stringify({ script: 'bun test', description, intervalMs: 15_000 }),
    output: [{ type: 'text', text: `status: ${end.status === 'lost' ? `lost: ${end.lostReason}` : end.status}\ncommandId: ${commandId}\noutput:\n` }],
    view: shellView({ commandId, ...end, chunks: [] }),
  })
  return [
    run('end-short', '31', 'Run the first short sleep', { status: 'exited', exitCode: 0 }),
    run('end-long', '32', 'Run the whole test suite and collect coverage for every package in the workspace', { status: 'running' }),
    run('end-failed', '33', 'Run the login test', { status: 'exited', exitCode: 1 }),
    run('end-stopped', '34', 'Start the dev server', { status: 'aborted' }),
    run('end-lost', '37', 'Run the deploy', { status: 'lost', lostReason: 'Demi was upgraded and the Host\'s runner replaced itself' }),
    lookCall('end-look', 'Check the first short sleep', '31', '35', statusText('31', 'exited', 'slept 2 s\n')),
    lookCall('end-look-several', 'Check the test suite and the dev server', '32 34', '36', statusText('32', 'running', '[412/980] packages/web\n')),
    toolCall({
      id: 'end-repeated',
      toolName: 'shell',
      status: 'error',
      input: JSON.stringify({ script: 'demi shell status 32', description: 'Check the test suite again', intervalMs: 15_000 }),
      output: [{ type: 'text', text: 'Repeated identical shell call suppressed.\nThe same script has been run 7 consecutive times in this agent session.\nInspect the previous output, use a different command, or provide the final answer instead of repeating it.' }],
      view: { kind: 'repeated_shell', script: 'demi shell status 32', count: 7 },
    }),
    toolCall({
      id: 'end-refused',
      toolName: 'shell',
      status: 'error',
      input: JSON.stringify({ script: 'bun run lint', intervalMs: 15_000 }),
      output: [{ type: 'text', text: 'shell input is invalid:\nmissing field `description`' }],
    }),
  ]
}

/** A report of `commandId`, whose call is titled `title`. */
function report(commandId: string, title: string, event: CommandReport['event'], output = ''): CommandReport {
  return { commandId, title, event, output }
}

const suite = 'Run the test suite'

/**
 * The reports the report-row specimens show, one of each event (`runtime.md`
 * § Command reports), by the specimen's variant.
 */
export function commandReportCases(): { variant: string, reports: CommandReport[] }[] {
  return [
    { variant: 'ended', reports: [report('17', suite, { kind: 'ended', exitCode: 0 }, '980 passed (6m 2s)')] },
    { variant: 'ended with a failure', reports: [report('17', suite, { kind: 'ended', exitCode: 1 }, 'FAIL auth.test.ts')] },
    { variant: 'stopped by you', reports: [report('18', 'Start the dev server', { kind: 'stopped', by: { kind: 'user' } })] },
    { variant: 'stopped by another agent', reports: [report('19', 'Watch the type check', { kind: 'stopped', by: { kind: 'agent', number: 2 } })] },
    { variant: 'lost', reports: [report('20', 'Serve the docs', { kind: 'lost', reason: 'its Host’s connection ended' })] },
    { variant: 'a call whose title is not known', reports: [report('21', '', { kind: 'ended', exitCode: 2 })] },
    {
      variant: 'ended, with the media its job viewed',
      reports: [{
        ...report('22', 'Capture the checkout screens', { kind: 'ended', exitCode: 0 }, `${imageLine(1, 480, 300, 15_822)}\n${imageLine(2, 480, 300, 16_078)}`),
        media: [blobImage(galleryBlobs.screenshot), blobImage(galleryBlobs.chart)],
      }],
    },
  ]
}

/**
 * Reports that arrived together, one block: a row for each end, none for the
 * progress among them; the last title is too long for a narrow row.
 */
export function severalReports(): CommandReport[] {
  return [
    report('17', suite, { kind: 'ended', exitCode: 1 }, 'FAIL auth.test.ts'),
    report('23', 'Watch the type check', { kind: 'running', runningMs: 300_000, idleMs: 0, intervalMs: 300_000 }),
    report('18', 'Start the dev server', { kind: 'stopped', by: { kind: 'user' } }),
    report('22', 'Build the documentation site with every locale and the API reference', { kind: 'ended', exitCode: 0 }, 'built 14 locales'),
  ]
}

/**
 * Calls of tools the runtime no longer has, as a conversation from before
 * they were removed keeps them: a look and a wait, each a generic tool card
 * (`runtime.md` § Rendering boundary).
 */
export function removedToolBlocks(): ToolCallBlock[] {
  return [
    toolCall({
      id: 'removed-status',
      toolName: 'shell_status',
      status: 'completed',
      input: JSON.stringify({ commandId: 17 }),
      output: [{ type: 'text', text: statusText('17', 'running', '✓ auth.test.ts > signs in (12 ms)\n') }],
      view: shellView({ commandId: '17', status: 'running', chunks: [] }),
    }),
    toolCall({
      id: 'removed-yield',
      toolName: 'yield',
      status: 'completed',
      input: JSON.stringify({ durationMs: 600_000, commandIds: [17] }),
      output: [{ type: 'text', text: 'yield scheduled\ndurationMs: 600000\ncommandIds: 17' }],
    }),
  ]
}

/** One request and one answer: enough history for a failure to keep on screen. */
export function shortTranscriptBlocks(): Block[] {
  return [
    {
      type: 'user',
      id: 'short-user',
      turnId: 'short-turn',
      createdAt: iso(120_000),
      model: demoModel,
      content: [
        {
          type: 'text',
          text: 'Build a small minesweeper game with a timer and difficulty settings.',
        },
      ],
      preamble: null,
    },
    {
      type: 'text',
      id: 'short-answer',
      createdAt: iso(60_000),
      model: demoModel,
      text: 'The game is ready.\n\nChoose a difficulty, reveal cells and mark suspected mines. The first click is always safe, and the timer starts with the first move.',
    },
  ]
}

/** The provider error that ends a turn, as the transcript records it. */
export function generationErrorBlock(): Block {
  return {
    type: 'error',
    id: 'generation-error',
    createdAt: iso(30_000),
    model: demoModel,
    message: 'Anthropic API request failed with HTTP 529: Overloaded. The provider could not accept the request.',
    code: 'overloaded',
    diagnostics: {
      source: 'http',
      httpStatus: 529,
      providerCode: 'overloaded_error',
      clientRequestId: 'req_01J8Y3Q6ZKX5',
    },
  }
}

/**
 * The record of a turn a Claude subscription's session limit refused, which
 * resets at `resetsAt`: the CLI's words, which name the hour in UTC, and as
 * the record the assistant line the CLI made up for the refusal, which
 * carries the reset, then its `result` line, which names the cause.
 */
export function usageLimitErrorBlock(id: string, resetsAt: string): Block {
  const hour = new Date(resetsAt).getUTCHours()
  const message = `You’ve hit your session limit · resets ${hour % 12 || 12}${hour < 12 ? 'am' : 'pm'} (UTC)`
  const sessionId = 'b4d1c0de-5e55-4a1e-9d1a-0c3f2a6b7e81'
  const notice = JSON.stringify({
    type: 'assistant',
    message: { model: '<synthetic>', role: 'assistant', content: [{ type: 'text', text: message }] },
    session_id: sessionId,
    error: 'rate_limit',
    is_api_error_message: true,
    api_error: 'usage_limit_reached',
    api_error_params: {
      rate_limit_info: {
        status: 'rejected',
        resetsAt: Math.floor(new Date(resetsAt).getTime() / 1000),
        rateLimitType: 'five_hour',
        isUsingOverage: false,
      },
    },
  })
  const result = JSON.stringify({
    type: 'result',
    subtype: 'success',
    is_error: true,
    api_error_status: 429,
    api_error: 'usage_limit_reached',
    result: message,
    session_id: sessionId,
  })
  return {
    type: 'error',
    id,
    createdAt: iso(30_000),
    model: demoModel,
    message,
    code: 'usage_limit',
    diagnostics: {
      source: 'stream',
      httpStatus: 429,
      upstream: `${notice}\n${result}`,
    },
  }
}

/**
 * The line an agent's move to another account leaves when its account
 * reached its 5-hour session limit, which resets at `resetsAt`, and the
 * answer the same request then got from the next account
 * (`providers.md` § Account rotation).
 */
export function accountSwitchBlocks(resetsAt: string): Block[] {
  return [
    {
      type: 'account_switch',
      id: 'account-switch',
      createdAt: iso(30_000),
      model: demoModel,
      from: 'alex@example.com',
      to: 'sam@example.com',
      window: '5h session',
      resetsAt,
    },
    {
      type: 'text',
      id: 'account-switch-answer',
      createdAt: iso(34_000),
      model: demoModel,
      text: 'The board now opens every empty neighbor when a blank cell is revealed.',
    },
  ]
}

/**
 * The record a turn ends with when its Host went offline and no later call
 * ran there: Demi's fact, which leaves the turn unfinished, and Resume waits
 * for the device.
 */
export function hostOfflineErrorBlock(): Block {
  return {
    type: 'error',
    id: 'host-offline-error',
    createdAt: iso(30_000),
    model: demoModel,
    message: 'MacBook Pro went offline, so this turn could not finish its work.',
    code: 'host_offline',
    device: { id: 'mac', name: 'MacBook Pro' },
  }
}

/** A steer typed while the turn runs. Renders after every transcript block, never among them. */
export function transcriptDemoBlocks(): Block[] {
  const thinkingStartedAt = iso(18_000)
  const thinkingEndedAt = iso(10_000)

  return [
    {
      type: 'user',
      id: 'user-1',
      turnId: 'turn-1',
      createdAt: iso(120_000),
      model: demoModel,
      content: userPrompt,
      preamble: null,
    },
    ...inRun(
      {
        type: 'thinking',
        id: 'thinking-streaming',
        createdAt: thinkingStartedAt,
        model: demoModel,
        text: thinkingText,
        signature: null,
      },
      {
        type: 'thinking',
        id: 'thinking-done',
        createdAt: thinkingStartedAt,
        model: demoModel,
        text: thinkingText,
        signature: null,
      },
      shellTool as Block,
      screenshotTool as Block,
      ...agentReceiptMessages.slice(0, 2).map((message): Block => ({
        type: 'agent_message', id: message.id, turnId: 'turn-1',
        createdAt: message.timestamp, model: demoModel, message,
      })),
    ),
    {
      type: 'steer',
      id: 'steer-1',
      turnId: 'turn-1',
      createdAt: iso(100_000),
      model: demoModel,
      content: [
        {
          type: 'text',
          text: 'Also add a case for the expired cookie.'
        }
      ],
    },
    ...inRun(
      runningShellTool as Block,
      editingShellTool as Block,
      lookTool as Block,
      inputTool as Block,
      stopTool as Block,
      withLists(errorTool as Block, listedFiles(editingShellTool)),
    ),
    {
      type: 'compaction_boundary',
      id: 'compaction-done',
      createdAt: iso(8_000),
      model: demoModel,
      summary: 'Kept the cookie helper and the new session assertion.',
      summaryTokens: 2400,
      kept: [],
    },
    {
      type: 'text',
      id: 'assistant-1',
      createdAt: thinkingEndedAt,
      model: demoModel,
      text: assistantMarkdown,
    },
    {
      type: 'abort',
      id: 'abort-1',
      createdAt: iso(2_000),
      model: demoModel,
      isResumed: false,
    },
    {
      type: 'error',
      id: 'error-1',
      createdAt: iso(1_000),
      model: demoModel,
      message: 'Anthropic API request failed with HTTP 429: This request would exceed the rate limit of 50 requests per minute for your organization. Retry after 12 seconds.',
      code: 'rate_limit',
      diagnostics: {
        source: 'http',
        httpStatus: 429,
        providerCode: 'rate_limit_error',
        clientRequestId: 'req_01J8Y3Q6ZKX4',
      },
    },
    {
      type: 'user',
      id: 'user-attachments',
      turnId: 'turn-1',
      createdAt: iso(900),
      model: demoModel,
      content: [
        { type: 'image', source: { type: 'url', url: demoImageUrl } },
        {
          type: 'document',
          source: {
            type: 'ref',
            ref: galleryBlobs.guide,
            mediaType: 'application/pdf',
            fileName: 'login-failure.pdf'
          }
        },
        {
          type: 'reference',
          reference: encodeRemoteReference('alex-mbp', '/Users/alex/Projects/shop/package.json')
        },
        { type: 'text', text: 'Failing log and the screenshot from CI.' },
      ],
      preamble: null,
    },
  ]
}

/**
 * A question the agent thought about, then answered: its thinking is a row
 * of its own, which a page read after a reload holds without its text
 * (`web-api.md` § Light form).
 */
export function thoughtAnswerBlocks(): Block[] {
  return [
    {
      type: 'user', id: 'thought-ask', turnId: 'thought-ask-turn', createdAt: iso(120_000), model: demoModel,
      content: [{ type: 'text', text: 'Is 1001 prime?' }], preamble: null,
    },
    ...inRun({
      type: 'thinking', id: 'thought-think', createdAt: iso(119_000), model: demoModel,
      text: '1001 = 7 × 11 × 13, so it is not prime.', signature: null,
    }),
    { type: 'text', id: 'thought-answer', createdAt: iso(114_000), model: demoModel, text: 'No: 1001 is 7 × 11 × 13.' },
  ]
}

/**
 * Runs that only check a command, and ones that run one too (`runtime.md`
 * § Work groups): the agent starts the end-to-end suite and ends its turn;
 * asked, it checks the suite twice; the suite's end report wakes it, a row
 * of its own, and carries the suite's last lines, so the agent answers
 * without a look; asked again, it runs the suite once more and checks the
 * first run's log. The groups read Checked 1 command and Ran 1 command,
 * checked 1 command.
 */
export function commandLookBlocks(): Block[] {
  const user = (id: string, at: number, text: string): Block => ({
    type: 'user', id, turnId: `${id}-turn`, createdAt: iso(at), model: demoModel,
    content: [{ type: 'text', text }], preamble: null,
  })
  const think = (id: string, at: number, text: string): Block => ({
    type: 'thinking', id, createdAt: iso(at), model: demoModel, text, signature: null,
  })
  const reply = (id: string, at: number, text: string): Block => ({
    type: 'text', id, createdAt: iso(at), model: demoModel, text,
  })
  const run = (id: string, at: number, commandId: string): Block => toolCall({
    id,
    createdAt: iso(at),
    toolName: 'shell',
    status: 'completed',
    input: JSON.stringify({ script: 'bun run e2e', description: 'Run the end-to-end suite', intervalMs: 300_000 }),
    view: shellView({ commandId, status: 'running', chunks: [{ stream: 'stdout', text: 'Running 140 specs\n' }] }),
  })
  const look = (id: string, at: number, description: string, ownCommandId: string, seen: string): Block =>
    lookCall(id, description, '51', ownCommandId, seen, at)
  const report: Block = {
    type: 'wakeup', id: 'look-report', turnId: 'look-report-turn', createdAt: iso(300_000), model: demoModel, placement: 'new_turn',
    reports: [{ commandId: '51', title: 'Run the end-to-end suite', event: { kind: 'ended', exitCode: 0 }, output: '140 passed (4m 12s)' }],
  }
  return [
    user('look-start', 900_000, 'Run the end-to-end suite.'),
    ...inRun(run('look-suite', 890_000, '51')),
    reply('look-started', 880_000, 'The suite is running; it takes a few minutes. I will tell you when it ends.'),
    user('look-ask', 700_000, 'Is it done?'),
    ...inRun(
      think('look-think', 690_000, 'Look at how far the suite got.'),
      look('look-first', 685_000, 'Check the end-to-end suite', '53', statusText('51', 'running', '[84/140] checkout.spec.ts\n')),
      think('look-think-again', 680_000, 'It is on the checkout specs. Look once more before answering.'),
      look('look-second', 675_000, 'Check the end-to-end suite again', '54', statusText('51', 'running', '[86/140] checkout.spec.ts\n')),
    ),
    reply('look-answer', 670_000, 'Not yet: 86 of 140 specs passed so far, and none failed. I will tell you when it ends.'),
    ...inRun(report),
    reply('report-answer', 295_000, 'The suite ended: all 140 specs passed in 4 minutes 12 seconds.'),
    user('again-ask', 200_000, 'Run it once more, and show me how long the first run took.'),
    ...inRun(
      run('again-suite', 195_000, '52'),
      look('again-look', 190_000, 'Read the first run’s timing', '56', statusText('51', 'exited', '140 passed (4m 12s)\n')),
    ),
    reply('again-answer', 185_000, 'The first run took 4 minutes 12 seconds. The second one is running; I will tell you when it ends.'),
  ]
}

/** The call whose command the running run's specimen runs (`runningRunBlocks`). */
export const RUNNING_RUN_CALL = 'running-run-merge'

/**
 * A turn whose run still goes (`runtime.md` § Work groups): it thought,
 * checked the branch, and its newest call has merged and run the tests for
 * a minute and a half. Folded, the run's row is that call and wears the time
 * its command has run, as the call's own row does; open, it says what runs
 * and for how long. The gallery's `runningRunCommand` is that command.
 */
export function runningRunBlocks(): Block[] {
  const shell = (id: string, at: number, script: string, description: string, done: boolean): Block => toolCall({
    id,
    createdAt: iso(at),
    toolName: 'shell',
    status: done ? 'completed' : 'executing',
    input: JSON.stringify({ script, description }),
    view: done ? shellView({ commandId: '61', runningMs: 400, chunks: [{ stream: 'stdout', text: 'On branch fix/reset-wording\nnothing to commit, working tree clean\n' }] }) : null,
  })
  return [
    {
      type: 'user', id: 'running-run-ask', turnId: 'running-run-turn', createdAt: iso(100_000), model: demoModel,
      content: [{ type: 'text', text: 'Merge the reset wording and run the tests.' }], preamble: null,
    },
    ...inRun(
      { type: 'thinking', id: 'running-run-think', createdAt: iso(98_000), model: demoModel, text: 'Check the branch is clean, then merge and run the suite.', signature: null },
      shell('running-run-status', 96_000, 'git status', 'Check the branch', true),
      shell(RUNNING_RUN_CALL, 92_000, 'git merge --no-ff fix/reset-wording && bun run test', 'Review and merge the reset wording', false),
    ),
  ]
}

/** The stages a run steps through (`steppedRunBlocks`), the last being the answer. */
export const STEPPED_RUN_STAGES = [
  'one step',
  'two steps',
  'the step ended, the model is waited for',
  'the next step',
  'the answer',
] as const

/** The steps of the stepped run in the order they come: its call ids, scripts and descriptions. */
const STEPPED_RUN_CALLS = [
  { id: 'stepped-run-ls', script: 'ls tests/', description: 'List the tests', output: 'auth.test.ts\nsession.test.ts\n', exitCode: 0 },
  { id: 'stepped-run-test', script: 'bun test auth', description: 'Run the auth tests', output: '1 fail 3 pass\n', exitCode: 1 },
  { id: 'stepped-run-chmod', script: 'chmod +x scripts/run.sh', description: 'Make the script executable', output: '', exitCode: 0 },
]

/** The call that runs at `stage` of the stepped run: the first, second and third call at stages 0, 1 and 3; none while the model is waited for, or after the answer. */
export function steppedRunCall(stage: number): (typeof STEPPED_RUN_CALLS)[number] | null {
  const running = [0, 1, null, 2, null][stage]
  return running === null || running === undefined ? null : STEPPED_RUN_CALLS[running]!
}

/**
 * A running turn's run at `stage` of `STEPPED_RUN_STAGES` (`runtime.md`
 * § Work groups), as the transcript shows it while the run goes: one step
 * runs, its own row; it returned and a second runs under it; the second
 * returned failed and joined the run, Requesting under it while the model
 * is waited for; a third runs under the run; and the answer, after the third
 * joined. Its newest running call is `steppedRunCall`'s.
 */
export function steppedRunBlocks(stage: number): Block[] {
  const called = [1, 2, 2, 3, 3][stage]!
  const calls = STEPPED_RUN_CALLS.slice(0, called).map((call, index) => {
    // The newest call runs at the stage that names it; an earlier one, and one the model is waited for, has ended.
    const running = steppedRunCall(stage)?.id === call.id
    return toolCall({
      id: call.id,
      createdAt: iso(60_000 - index * 15_000),
      toolName: 'shell',
      status: running ? 'executing' : 'completed',
      input: JSON.stringify({ script: call.script, description: call.description }),
      view: running ? null : shellView({ commandId: `8${index}`, runningMs: 1_800, exitCode: call.exitCode, chunks: [{ stream: 'stdout', text: call.output }] }),
    })
  })
  const blocks: Block[] = [
    {
      type: 'user', id: 'stepped-run-ask', turnId: 'stepped-run-turn', createdAt: iso(70_000), model: demoModel,
      content: [{ type: 'text', text: 'Fix the failing auth tests.' }], preamble: null,
    },
    ...inRun(...calls),
  ]
  if (stage === 4)
    blocks.push({ type: 'text', id: 'stepped-run-answer', createdAt: iso(1_000), model: demoModel, text: 'The auth tests pass now: the run script was not executable.' })
  return blocks
}

/** The call whose command runs on after its turn ended (`leftRunningBlocks`). */
export const LEFT_RUNNING_CALL = 'left-running-suite'

/**
 * A turn that ended while a command it started runs on (`runtime.md`
 * § Work groups): the agent thought, started the suite, and replied without
 * waiting. The ended run's row says what it did and wears the time the
 * command has run until it ends. The gallery's `leftRunningCommand` is it.
 */
export function leftRunningBlocks(): Block[] {
  return [
    {
      type: 'user', id: 'left-running-ask', turnId: 'left-running-turn', createdAt: iso(140_000), model: demoModel,
      content: [{ type: 'text', text: 'Start the end-to-end suite and tell me when it ends.' }], preamble: null,
    },
    ...inRun(
      { type: 'thinking', id: 'left-running-think', createdAt: iso(138_000), model: demoModel, text: 'The suite takes minutes: start it and let it report.', signature: null },
      // Screenshots of the app's screens the run took first: the folded row holds their pictures and the command's time.
      screenshotsCall(
        {
          id: 'left-running-shots',
          createdAt: iso(137_000),
          toolName: 'shell',
          input: JSON.stringify({
            script: 'for t in t1 t2 t3 t4 t5 t6 t7 t8 t9 t10 t11 t12; do demi browser screenshot "$t" | demi file view; done',
            description: 'Take a screenshot of each screen the suite covers',
          }),
        },
        'cmd-left-running-shots',
        captures(12),
      ),
      toolCall({
        id: LEFT_RUNNING_CALL,
        createdAt: iso(136_000),
        toolName: 'shell',
        status: 'completed',
        input: JSON.stringify({ script: 'bun run e2e', description: 'Run the end-to-end suite', intervalMs: 15_000 }),
        view: shellView({ commandId: '71', status: 'running', runningMs: 15_000, chunks: [{ stream: 'stdout', text: 'Running 140 specs\n' }] }),
      }),
    ),
    { type: 'text', id: 'left-running-reply', createdAt: iso(120_000), model: demoModel, text: 'The suite is running; I will tell you when it ends.' },
  ]
}

/**
 * A run that viewed two screenshots, then a report that carried two
 * (`file-previews.md` § Media a tool returned): folded, the run's row shows
 * its call's screenshots under it, and the report's row its own.
 */
export function viewedMediaBlocks(): Block[] {
  const user = (id: string, at: number, text: string): Block => ({
    type: 'user', id, turnId: `${id}-turn`, createdAt: iso(at), model: demoModel,
    content: [{ type: 'text', text }], preamble: null,
  })
  const reply = (id: string, at: number, text: string): Block => ({
    type: 'text', id, createdAt: iso(at), model: demoModel, text,
  })
  const open = toolCall({
    id: 'viewed-open',
    createdAt: iso(590_000),
    toolName: 'shell',
    status: 'completed',
    input: JSON.stringify({ script: 'demi browser open https://example.com', description: 'Open example.com' }),
    output: [{ type: 'text', text: 'status: exited\nexitCode: 0\ncommandId: 61\noutput:\nt1\n' }],
    view: shellView({ commandId: '61', chunks: [{ stream: 'stdout', text: 't1\n' }] }),
  })
  const shots = screenshotsCall(
    {
      id: 'viewed-shots',
      createdAt: iso(585_000),
      toolName: 'shell',
      input: JSON.stringify({
        script: 'for i in 1 2; do demi browser screenshot t1 | demi file view; done',
        description: 'Take two screenshots of example.com',
      }),
    },
    '62',
    [
      { tab: 't1', width: 480, height: 300, bytes: 15_822, medium: blobImage(galleryBlobs.screenshot) },
      { tab: 't1', width: 480, height: 300, bytes: 16_078, medium: blobImage(galleryBlobs.chart) },
    ],
  )
  const report: Block = {
    type: 'wakeup', id: 'viewed-report', turnId: 'viewed-report-turn', createdAt: iso(300_000), model: demoModel, placement: 'new_turn',
    reports: [{
      commandId: '63',
      title: 'Capture the checkout screens',
      event: { kind: 'ended', exitCode: 0 },
      output: `${imageLine(1, 480, 300, 15_822)}\n${imageLine(2, 480, 300, 16_078)}`,
      media: [blobImage(galleryBlobs.screenshot), blobImage(galleryBlobs.chart)],
    }],
  }
  return [
    user('viewed-ask', 600_000, 'Open example.com and take two screenshots of it.'),
    ...inRun(open, shots),
    reply('viewed-answer', 580_000, 'Both screenshots show the same page: a heading and one link. I am capturing the checkout screens too and will tell you when that ends.'),
    ...inRun(report),
    reply('viewed-report-answer', 295_000, 'The checkout screens are captured: the cart and the payment step.'),
  ]
}

/**
 * A turn whose model has finished its sentence and goes on writing a call
 * (`runtime.md` § Calls being written): the user's message and the finished
 * text, which the specimens follow with a call being written or with
 * Requesting.
 */
export function callBeingWrittenBlocks(): Block[] {
  return [
    {
      type: 'user',
      id: 'writing-user',
      turnId: 'writing-turn',
      createdAt: '2026-10-08T12:00:00.000Z',
      model: demoModel,
      content: [{ type: 'text', text: 'Sort the downloads into folders by kind.' }],
      preamble: null,
    },
    {
      type: 'text',
      id: 'writing-text',
      createdAt: '2026-10-08T12:00:04.000Z',
      model: demoModel,
      text: 'Now I have everything needed to classify. Let me write a categorizer.',
      forkable: true,
    },
  ]
}

/**
 * One request of real work, as a model does it: short sentences between
 * single commands, thinking before most of them, a run of reads, a failing
 * test, an edit and a summary, so the transcript's rows and its prose take
 * turns all the way down. `working` stops it at the last command, still
 * running, as the page shows a request in progress.
 */
export function busyRequestBlocks(working = false): Block[] {
  let at = 600_000
  const next = () => (at -= 9_000)
  const think = (id: string, text: string): Block => ({
    type: 'thinking', id: `busy-${id}`, createdAt: iso(next()), model: demoModel, text, signature: null,
  })
  const say = (id: string, text: string): Block => ({
    type: 'text', id: `busy-${id}`, createdAt: iso(next()), model: demoModel, text,
  })
  const run = (
    id: string,
    description: string,
    script: string,
    output: string,
    extra: Partial<ShellView> = {},
  ): ToolCallBlock => toolCall({
    id: `busy-${id}`,
    createdAt: iso(next()),
    toolName: 'shell',
    status: 'completed',
    input: JSON.stringify({ script, description, intervalMs: 60_000 }),
    output: [{ type: 'text', text: output }],
    view: shellView({ commandId: `busy-${id}`, chunks: [{ stream: 'stdout', text: output }], ...extra }),
  })
  const blocks: Block[] = [
    {
      type: 'user', id: 'busy-user', turnId: 'busy-turn', createdAt: iso(at), model: demoModel,
      content: [{ type: 'text', text: 'The checkout page times out on slow networks. Find out why and fix it.' }],
      preamble: null,
    },
    ...inRun(think('think-1', 'Start where the checkout request sets its timeout, then see what retries it.')),
    say('say-1', 'I will start with where the checkout request sets its timeout.'),
    ...inRun(run('find', 'Find the checkout timeout', 'rg -n "timeout" packages/web/src/checkout', 'packages/web/src/checkout/api.ts:14:  timeout: 5_000,\n')),
    say('say-2', 'The request gives up after 5 seconds, in `checkout/api.ts`. Next, how the retries interact with it.'),
    ...inRun(
      think('think-2', 'If the retry helper retries a timed-out request with the same budget, a slow network never gets past the first attempt.'),
      run('read-retry', 'Read the retry helper', 'sed -n 1,60p packages/web/src/net/retry.ts', 'export async function withRetry(request, { attempts = 3 }) {\n'),
      run('read-client', 'Read the checkout client', 'sed -n 1,80p packages/web/src/checkout/api.ts', 'export const checkout = (cart) => withRetry(() => post("/checkout", cart, { timeout: 5_000 }))\n'),
    ),
    say('say-3', `Found it. Each retry starts a fresh 5-second timeout, but the server answers a slow checkout with **504** after 8 seconds, and \`withRetry\` does not retry a 504, only a timeout.

So on a slow network every attempt times out before the server can answer, and the page shows the timeout error after three tries.`),
    ...inRun(
      think('think-3', 'Confirm with the tests before changing anything.'),
      run('test', 'Run the checkout tests', 'bun test packages/web/src/checkout', ' 11 pass\n 1 fail\n✗ checkout > survives a slow network\n', { exitCode: 1 }),
    ),
    say('say-4', 'One test already covers this, and it fails the same way. I will raise the timeout above the server\'s 8 seconds and retry a 504.'),
  ]
  const edit = ownLists(toolCall({
      id: 'busy-edit',
      createdAt: iso(next()),
      toolName: 'shell',
      status: 'completed',
      input: JSON.stringify({ script: 'apply_patch <<\'EOF\'\n…\nEOF', description: 'Raise the timeout and retry a 504', intervalMs: 60_000 }),
      view: shellView({
        commandId: 'busy-edit',
        chunks: [{ stream: 'stdout', text: 'Updated 2 files\n' }],
        files: [
          editedFile({ path: 'packages/web/src/checkout/api.ts', kind: 'modified', added: 2, removed: 1 }),
          editedFile({ path: 'packages/web/src/net/retry.ts', kind: 'modified', added: 4, removed: 1 }),
        ],
      }),
    }))
  const finalTests = withLists(run('test-again', 'Run the checkout tests again', 'bun test packages/web/src/checkout', ' 12 pass\n 0 fail\n'), listedFiles(edit))
  if (working) {
    return [
      ...blocks,
      ...inRun(edit, { ...finalTests, status: 'executing', output: [], view: shellView({ commandId: 'busy-test-again', status: 'running', chunks: [{ stream: 'stdout', text: ' 7 pass\n' }] }) }),
    ]
  }
  return [
    ...blocks,
    ...inRun(edit, finalTests),
    say('say-5', `Fixed. The checkout page now waits through a slow network:

- The request's timeout is 10 seconds, above the 8 seconds after which the server answers a slow checkout.
- \`withRetry\` retries a **504** as it retries a timeout.

All 12 checkout tests pass, including the slow-network one that failed before.`),
  ]
}

/** The suites the found-mark transcript's run starts, by the command each runs as. */
const FOUND_SUITES = [
  ['61', 'Run the parser tests'],
  ['62', 'Run the interpreter tests'],
  ['63', 'Run the regexp tests'],
  ['64', 'Run the garbage collector tests'],
  ['65', 'Run mjsunit with JIT translation on'],
  ['66', 'Run the wasm spec tests'],
  ['67', 'Run the inspector tests'],
] as const

/**
 * A transcript to bring blocks into view in: a short call, an answer taller
 * than a session's view, a short answer after it, and a run of seven
 * commands, folded, one of which then reports its end in a notice whose
 * title brings its call into view.
 */
export function foundMarkBlocks(): Block[] {
  const user = (id: string, at: number, text: string): Block => ({
    type: 'user', id, turnId: `${id}-turn`, createdAt: iso(at), model: demoModel,
    content: [{ type: 'text', text }], preamble: null,
  })
  const reply = (id: string, at: number, text: string): Block => ({
    type: 'text', id, createdAt: iso(at), model: demoModel, text,
  })
  const steps = [
    'The session cookie was renamed from `sid` to `session` in the cookie helper, and the login test still reads the old name.',
    'The helper writes the cookie once per sign-in, and every request after it reads the cookie back through `readSessionCookie`.',
    'The test drives the login page, submits the form, and reads the `Set-Cookie` header of the response.',
    'Its assertion compares the parsed cookie with a literal that names `sid`, so it fails while the page itself works.',
    'I changed the literal to `session` and left the helper alone, as you asked.',
    'The expired-cookie case now has a test of its own: it signs in, moves the clock past the cookie’s age, and expects the sign-in page again.',
    'Both tests pass, and the whole suite passes in 41 seconds.',
  ]
  return [
    user('found-ask', 600_000, 'Why does the login test fail?'),
    { ...shellTool, id: 'found-call', createdAt: iso(590_000) },
    reply('found-long', 580_000, steps.map((step) => `${step} ${'Each step follows from the one before it, and the next one reads its result. '.repeat(3)}`).join('\n\n')),
    user('found-thanks', 100_000, 'Thanks. Is anything left?'),
    reply('found-short', 90_000, 'Nothing is left: both tests pass.'),
    user('found-suites', 80_000, 'Run the engine’s suites.'),
    ...inRun(...FOUND_SUITES.map(([commandId, description], index) => toolCall({
      id: `found-suite-${commandId}`,
      createdAt: iso(70_000 - index * 1_000),
      toolName: 'shell',
      status: 'completed',
      input: JSON.stringify({ script: `tools/run-tests.py --suite=${commandId}`, description }),
      view: shellView({ commandId, status: 'running', chunks: [{ stream: 'stdout', text: 'Running…\n' }] }),
    }))),
    reply('found-started', 60_000, 'The seven suites are running. I will tell you as each ends.'),
    {
      type: 'wakeup', id: 'found-report', turnId: 'found-report-turn', createdAt: iso(30_000), model: demoModel, placement: 'new_turn',
      reports: [{ commandId: '65', title: 'Run mjsunit with JIT translation on', event: { kind: 'ended', exitCode: 0 }, output: '5,812 tests passed' }],
    },
  ]
}
