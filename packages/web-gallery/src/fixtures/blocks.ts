import type { AgentMessage, Block, CommandReport, EditedFile, ModelSelection, ToolResultContentBlock, UserContentBlock } from '@demicodes/protocol'
import { encodeRemoteReference } from '@demicodes/web-ui/agent/message-input/attachments'
import type { ShellToolView as ShellView, ToolCallBlock } from '@demicodes/web-ui/agent/block-types'
import { editCopies, galleryBlobSize, galleryBlobs, missingBlob } from './blobs'

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

export const agentReceiptMessages: AgentMessage[] = [
  { type: 'message' as const },
  { type: 'completion' as const, outcome: 'completed' as const },
  { type: 'completion' as const, outcome: 'failed' as const },
  { type: 'completion' as const, outcome: 'aborted' as const },
].map((event, index) => ({
  id: event.type === 'completion' ? `subagent:agent-ui-${index}:1` : `receipt-${index}`,
  sender: { id: `agent-ui-${index}`, number: index + 1, description: 'UI implementation', round: 1 },
  recipientId: 'gallery-parent',
  timestamp: '2026-09-12T12:00:00.000Z',
  content: index === 0
    ? 'The shared receipt component is ready. I am checking **keyboard expansion** and reconnect behavior.'
    : index === 1
      ? ['Implemented the shared component and verified the product and gallery.', '', '## Validation', ...Array.from({ length: 12 }, (_, i) => `- Check ${i + 1}: source identity, content, and message order remain intact.`)].join('\n')
      : index === 2
        ? 'The fixture could not reach the test server. No files were changed.\n\n`ECONNREFUSED 127.0.0.1:3271`'
        : 'The parent stopped this round before validation finished.',
  event,
}))

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
  content: 'The user moved this conversation from Cloud (/home/demi/sessions/0f4e2a1c) to MacBook Pro (/Users/zan/code/ledable-app). Files did not move. Check what this means for the work so far, and tell the user.',
  event: { type: 'moved', host: 'MacBook Pro', path: '/Users/zan/code/ledable-app', home: '/Users/zan' },
}
/** A decision on a request of two categories, and Demi's notice that a move the agent asked for failed. */
export const organizeReceiptMessages: AgentMessage[] = [
  {
    id: 'permission:pr-move',
    recipientId: 'gallery-parent',
    timestamp: '2026-09-12T12:06:00.000Z',
    content:
      'The user allowed this conversation to organize conversations and manage devices; the command `demi conversation move ledable-app` can now run.',
    event: { type: 'permission' as const, outcome: 'allowed' as const, action: 'organize conversations and manage devices' },
  },
  {
    id: 'move-failed:4c1e9d2b',
    recipientId: 'gallery-parent',
    timestamp: '2026-09-12T12:08:00.000Z',
    content:
      "Demi could not make the move this conversation's agent asked for: the project no longer exists. The conversation still runs on Cloud (/home/demi/sessions/0f4e7597-e889-4baa-ac98-93fa61b68d28).",
    event: { type: 'move_failed' as const },
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

function toolCall(
  partial: Pick<ToolCallBlock, 'id' | 'toolName' | 'input' | 'status'> & Partial<ToolCallBlock>
): ToolCallBlock {
  return {
    type: 'tool_call',
    createdAt: iso(120_000),
    model: demoModel,
    toolUseId: `${partial.id}-use`,
    output: [],
    view: null,
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

export const thinkingText = `The cookie name changed from sid to session. The helper already writes the new header.

The test is the one still looking for sid, so the fix belongs in the test, not in the helper.

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
 * One long line of words with hyphens inside them and a signed address
 * longer than any line: the command wraps at its spaces, each word whole,
 * and only the address breaks inside, filling its lines to the edge.
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
  partial: Pick<ToolCallBlock, 'id' | 'toolName' | 'input'>,
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

/** An icon smaller than a thumbnail: it keeps its own size, never enlarged. */
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
export const fileChangeCases: { variant: string, block: ToolCallBlock }[] = [
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
    variant: 'more than three rows',
    block: fileChangeCase('many', 'Rename the prop across every widget', Array.from({ length: 40 }, (_, i) => editedFile({
      path: `packages/web/src/components/Widget${i + 1}.vue`,
      kind: i % 9 === 0 ? 'added' : 'modified',
      added: i + 1,
      removed: i % 3,
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
    caseBlock('two edits to one file'),
    text('changes-text-1', 150_000, 'The helper now writes `session`. Sweeping the widgets next; each one reads the cookie name from a prop.'),
    caseBlock('more than three rows'),
    caseBlock('a few, mixed'),
    caseBlock('no files'),
    caseBlock('long names'),
    caseBlock('append only, not new'),
    text('changes-text-2', 60_000, 'Every widget compiles and the snapshots match. Applying the same codemod to the docs examples.'),
    viewedCall(
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
    ),
    caseBlock('still running'),
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
 * stopped or still run, one with a title too long for a narrow row, looks
 * at them, and calls that ran nothing: the repeat guard's and a refused
 * input's.
 */
export function commandEndBlocks(): ToolCallBlock[] {
  type End = { status: 'exited', exitCode: number } | { status: 'running' | 'aborted' }
  const failed = (end: End) => end.status === 'aborted' || (end.status === 'exited' && end.exitCode !== 0)
  const run = (id: string, commandId: string, description: string, end: End) => toolCall({
    id,
    toolName: 'shell',
    status: failed(end) ? 'error' : 'completed',
    input: JSON.stringify({ script: 'bun test', description, intervalMs: 15_000 }),
    output: [{ type: 'text', text: `status: ${end.status}\ncommandId: ${commandId}\noutput:\n` }],
    view: shellView({ commandId, ...end, chunks: [] }),
  })
  return [
    run('end-short', '31', 'Run the first short sleep', { status: 'exited', exitCode: 0 }),
    run('end-long', '32', 'Run the whole test suite and collect coverage for every package in the workspace', { status: 'running' }),
    run('end-failed', '33', 'Run the login test', { status: 'exited', exitCode: 1 }),
    run('end-stopped', '34', 'Start the dev server', { status: 'aborted' }),
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
    { variant: 'still running', reports: [report('17', suite, { kind: 'running', runningMs: 300_000, idleMs: 12_000, intervalMs: 300_000 }, '[412/980] packages/web')] },
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

/** Reports that arrived together, one block, one row each; the last title is too long for a narrow row. */
export function severalReports(): CommandReport[] {
  return [
    report('17', suite, { kind: 'ended', exitCode: 1 }, 'FAIL auth.test.ts'),
    report('18', 'Start the dev server', { kind: 'stopped', by: { kind: 'user' } }),
    report('22', 'Build the documentation site with every locale and the API reference', { kind: 'running', runningMs: 600_000, idleMs: 0, intervalMs: 600_000 }),
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
    runningShellTool as Block,
    editingShellTool as Block,
    lookTool as Block,
    inputTool as Block,
    stopTool as Block,
    errorTool as Block,
    {
      type: 'compaction_boundary',
      id: 'compaction-done',
      createdAt: iso(8_000),
      model: demoModel,
      summary: 'Kept the cookie helper and the new session assertion.',
      summaryTokens: 2400,
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
          reference: encodeRemoteReference('zan-mbp', '/Users/zan/Projects/demi/package.json')
        },
        { type: 'text', text: 'Failing log and the screenshot from CI.' },
      ],
      preamble: null,
    },
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
    run('look-suite', 890_000, '51'),
    reply('look-started', 880_000, 'The suite is running; it takes a few minutes. I will tell you when it ends.'),
    user('look-ask', 700_000, 'Is it done?'),
    think('look-think', 690_000, 'Look at how far the suite got.'),
    look('look-first', 685_000, 'Check the end-to-end suite', '53', statusText('51', 'running', '[84/140] checkout.spec.ts\n')),
    think('look-think-again', 680_000, 'It is on the checkout specs. Look once more before answering.'),
    look('look-second', 675_000, 'Check the end-to-end suite again', '54', statusText('51', 'running', '[86/140] checkout.spec.ts\n')),
    reply('look-answer', 670_000, 'Not yet: 86 of 140 specs passed so far, and none failed. I will tell you when it ends.'),
    report,
    reply('report-answer', 295_000, 'The suite ended: all 140 specs passed in 4 minutes 12 seconds.'),
    user('again-ask', 200_000, 'Run it once more, and show me how long the first run took.'),
    run('again-suite', 195_000, '52'),
    look('again-look', 190_000, 'Read the first run’s timing', '56', statusText('51', 'exited', '140 passed (4m 12s)\n')),
    reply('again-answer', 185_000, 'The first run took 4 minutes 12 seconds. The second one is running; I will tell you when it ends.'),
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
