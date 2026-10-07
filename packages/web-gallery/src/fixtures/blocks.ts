import type { AgentMessage, Block, EditedFile, ModelSelection, ToolResultContentBlock, UserContentBlock } from '@demicodes/protocol'
import { encodeRemoteReference } from '@demicodes/web-ui/agent/message-input/attachments'
import type { ShellToolView as ShellView, ToolCallBlock } from '@demicodes/web-ui/agent/block-types'
import { editCopies, galleryBlobSize, galleryBlobs, missingBlob } from './blobs'

export type { ShellView }

/**
 * A complete shell view, the shape `shell_exec` writes and every reader
 * validates. A specimen names only the parts it is about; the rest is what a
 * finished command leaves behind.
 */
export function shellView(
  parts: Partial<ShellView> & Pick<ShellView, 'chunks'>
): ShellView {
  const view: ShellView = {
    kind: 'shell',
    status: 'exited',
    shellId: 'shell-demo',
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
  toolName: 'shell_exec',
  status: 'completed',
  input: JSON.stringify({
    script: 'rg -n "sid" packages/web/src',
    description: 'Find the old cookie name',
  }),
  view: shellView({
    commandId: 'cmd-find',
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

export const runningShellTool = toolCall({
  id: 'tool-shell-run',
  toolName: 'shell_exec',
  status: 'executing',
  input: JSON.stringify({
    // Long enough to wrap over more than two lines: the command shows two
    // above the output, which scrolls under it. Its flags' hyphens show the
    // wrap a terminal makes: each line fills to the box's edge.
    script: [
      'DEMI_LOG=auth=debug,cookie=debug,session=info bun test --watch --timeout 20000 --rerun-each 1 --bail 5 packages/web/src/auth.test.ts packages/web/src/cookie.test.ts packages/web/src/session.test.ts \\',
      '  2>&1 | tee target/auth-watch.log',
    ].join('\n'),
    description: 'Run the login test',
  }),
  view: shellView({
    commandId: 'cmd-run',
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
  toolName: 'shell_exec',
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

export const editingShellTool = toolCall({
  id: 'tool-shell-edit',
  toolName: 'shell_exec',
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

/** The line a command's output shows for its binary stdout (`runtime.md` § What a result attaches). */
function binaryLine(bytes: number): string {
  return `<binary stdout: ${bytes} bytes>\n`
}

/**
 * A shell call whose stdout is an image or a video (`runtime.md` § What a
 * result attaches): the result the model reads, with the medium, or the
 * part that took its place, between its status and its note, and the view
 * whose output names the binary stdout.
 */
function binaryStdoutCall(
  partial: Pick<ToolCallBlock, 'id' | 'toolName' | 'input'>,
  commandId: string,
  bytes: number,
  mediaType: string,
  medium: ToolResultContentBlock,
  files?: ShellView['files'],
): ToolCallBlock {
  return toolCall({
    ...partial,
    status: 'completed',
    output: [
      {
        type: 'text',
        text: `status: exited\nexitCode: 0\ncommandId: ${commandId}\noutput:\n${binaryLine(bytes)}`,
      },
      medium,
      { type: 'text', text: `Attached stdout as ${mediaType} (${bytes} bytes).` },
    ],
    view: shellView({
      commandId,
      chunks: [{ stream: 'stdout', text: binaryLine(bytes) }],
      ...(files ? { files } : {}),
    }),
  })
}

function blobImage(ref: string): ToolResultContentBlock {
  return { type: 'image', source: { type: 'ref', ref, mediaType: 'image/png', ...galleryBlobSize(ref) } }
}

const screenshotInput = JSON.stringify({
  script: 'demi browser screenshot t1',
  description: 'Take a screenshot of the login page',
})

/**
 * One screenshot a command returned as a medium: what it captured, its
 * bytes, and the part the result holds for it, its image or the part that
 * says it is gone.
 */
interface Capture {
  tab: string
  width: number
  height: number
  bytes: number
  medium: ToolResultContentBlock
}

/** The text a screenshot prints before its medium's line, as `demi browser screenshot` prints it. */
function captureText(capture: Capture): string {
  return `Screenshot of ${capture.tab}\nImage: ${capture.width} × ${capture.height} px, one pixel per CSS pixel\nViewport: ${capture.width} × ${capture.height} CSS px, device pixel ratio 2, web\n`
}

/**
 * A shell call whose declared commands returned images (`runtime.md`
 * § Media a command returns): the output holds each command's text and its
 * line `[medium n: …]`, and the result attaches the media after it, in
 * order, then the lines for those it did not attach.
 */
function returnedMediaCall(
  partial: Pick<ToolCallBlock, 'id' | 'toolName' | 'input'>,
  commandId: string,
  captures: Capture[],
  notes: string[] = [],
): ToolCallBlock {
  const output = captures
    .map((capture, index) => `${captureText(capture)}[medium ${index + 1}: image/png, ${capture.bytes} bytes]\n`)
    .join('')
  return toolCall({
    ...partial,
    status: 'completed',
    output: [
      { type: 'text', text: `status: exited\nexitCode: 0\ncommandId: ${commandId}\noutput:\n${output}` },
      ...captures.map((capture) => capture.medium),
      ...(notes.length > 0 ? [{ type: 'text' as const, text: notes.join('\n') }] : []),
    ],
    view: shellView({ commandId, chunks: [{ stream: 'stdout', text: output }] }),
  })
}

/** A screenshot the agent took, under its call (`file-previews.md` § Media a tool returned). */
export const screenshotTool = returnedMediaCall(
  { id: 'tool-screenshot', toolName: 'shell_exec', input: screenshotInput },
  '17',
  [{ tab: 't1', width: 480, height: 300, bytes: 15_822, medium: blobImage(galleryBlobs.screenshot) }],
)

/**
 * Three tabs in one call: a loop of screenshots, each image attached in the
 * loop's order; a fourth, the model could not read, is told of after them.
 */
export const screenshotsTool = returnedMediaCall(
  {
    id: 'tool-screenshots',
    toolName: 'shell_exec',
    input: JSON.stringify({
      script: 'for t in t1 t2 t3; do demi browser screenshot "$t"; done',
      description: 'Compare the login page in three tabs',
    }),
  },
  '18',
  [
    { tab: 't1', width: 480, height: 300, bytes: 15_822, medium: blobImage(galleryBlobs.screenshot) },
    { tab: 't2', width: 480, height: 300, bytes: 16_078, medium: blobImage(galleryBlobs.chart) },
    { tab: 't3', width: 360, height: 2400, bytes: 8_035, medium: blobImage(galleryBlobs.fullPage) },
  ],
  ['[medium 4: not attached: the model does not accept image/webp; save it: demi shell output 18 --medium 4 > <file>]'],
)

/** A capture of a whole page: taller than a thumbnail's proportions, so the thumbnail keeps its top. */
export const fullPageTool = binaryStdoutCall(
  {
    id: 'tool-full-page',
    toolName: 'shell_exec',
    input: JSON.stringify({
      script: 'demi browser screenshot t1 --full-page | convert - -strip png:-',
      description: 'Capture the whole login page',
    }),
  },
  'cmd-full-page',
  8_035,
  'image/png',
  blobImage(galleryBlobs.fullPage),
)

/** A timeline wider than a thumbnail's proportions: the thumbnail keeps its middle. */
export const wideImageTool = binaryStdoutCall(
  {
    id: 'tool-wide-image',
    toolName: 'shell_exec',
    input: JSON.stringify({
      script: 'cat out/timeline.png',
      description: 'Show the timeline of the run',
    }),
  },
  'cmd-timeline',
  5_883,
  'image/png',
  blobImage(galleryBlobs.timeline),
)

/** An icon smaller than a thumbnail: it keeps its own size, never enlarged. */
export const smallImageTool = binaryStdoutCall(
  {
    id: 'tool-small-image',
    toolName: 'shell_exec',
    input: JSON.stringify({
      script: 'cat public/favicon.png',
      description: 'Show the favicon',
    }),
  },
  'cmd-favicon',
  972,
  'image/png',
  blobImage(galleryBlobs.icon),
)

/**
 * A recording the agent printed: its first frame with a play mark, which a
 * click plays in the viewer.
 */
export const recordingTool = binaryStdoutCall(
  {
    id: 'tool-recording',
    toolName: 'shell_exec',
    input: JSON.stringify({
      script: 'cat out/checkout.webm',
      description: 'Show the recording of the checkout flow',
    }),
  },
  'cmd-record',
  23_336,
  'video/webm',
  {
    type: 'video',
    source: { type: 'ref', ref: galleryBlobs.recording, mediaType: 'video/webm', ...galleryBlobSize(galleryBlobs.recording) },
  },
)

/** A recording whose reference carries no size: its thumbnail is 16:9 until its first frame arrives. */
export const unsizedRecordingTool = binaryStdoutCall(
  {
    id: 'tool-recording-unsized',
    toolName: 'shell_exec',
    input: JSON.stringify({
      script: 'cat out/demo.mp4',
      description: 'Show the demo of the checkout',
    }),
  },
  'cmd-demo',
  31_822,
  'video/mp4',
  { type: 'video', source: { type: 'ref', ref: galleryBlobs.unsizedRecording, mediaType: 'video/mp4' } },
)

/** A command that exited after its call returned: the status call that saw the exit carries the picture. */
export const statusImageTool = binaryStdoutCall(
  {
    id: 'tool-status-image',
    toolName: 'shell_status',
    input: JSON.stringify({ commandId: 'cmd-chart', description: 'Check the latency chart' }),
  },
  'cmd-chart',
  16_078,
  'image/png',
  blobImage(galleryBlobs.chart),
)

/** A recording whose bytes could not be stored: in its place, the part that says so and why. */
export const notStoredVideoTool = binaryStdoutCall(
  {
    id: 'tool-recording-not-stored',
    toolName: 'shell_exec',
    input: JSON.stringify({
      script: 'cat out/checkout.webm',
      description: 'Show the recording of the checkout flow',
    }),
  },
  'cmd-record-lost',
  23_336,
  'video/webm',
  {
    type: 'gone',
    kind: 'video',
    mediaType: 'video/webm',
    cause: { type: 'not_stored', error: 'the object store refused the write' },
  },
)

/** A screenshot whose blob the page cannot load. */
export const missingImageTool = returnedMediaCall(
  { id: 'tool-screenshot-missing', toolName: 'shell_exec', input: screenshotInput },
  '11',
  [{ tab: 't1', width: 480, height: 300, bytes: 15_822, medium: blobImage(missingBlob) }],
)

function fileChangeCase(
  id: string,
  description: string,
  files: ShellView['files'],
  status: ToolCallBlock['status'] = 'completed',
): ToolCallBlock {
  return toolCall({
    id: `tool-files-${id}`,
    toolName: 'shell_exec',
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
    binaryStdoutCall(
      {
        id: 'changes-snapshot-diff',
        toolName: 'shell_exec',
        input: JSON.stringify({
          script: 'bun scripts/snapshot-diff.ts login && cat out/login-diff.png',
          description: 'Compare the login snapshot before and after',
        }),
      },
      'cmd-snapshot-diff',
      16_078,
      'image/png',
      blobImage(galleryBlobs.chart),
      [
        uncopiedFile({ path: 'out/login-diff.png', kind: 'added', added: 0, removed: 0 }),
        editedFile({ path: 'scripts/snapshot-diff.ts', kind: 'modified', added: 4, removed: 1 }),
      ],
    ),
    caseBlock('still running'),
  ]
}

export const yieldTool = toolCall({
  id: 'tool-yield',
  toolName: 'yield',
  status: 'completed',
  input: JSON.stringify({ description: 'Wait for the next user turn' }),
})

export const statusTool = toolCall({
  id: 'tool-status',
  toolName: 'shell_status',
  status: 'completed',
  input: JSON.stringify(
    {
      commandId: 'cmd_1',
      description: 'Check the dev server'
    }
  ),
})

export const writeTool = toolCall({
  id: 'tool-write',
  toolName: 'shell_write',
  status: 'completed',
  input: JSON.stringify({ commandId: 'cmd_1', data: 'continue' }),
})

export const abortTool = toolCall({
  id: 'tool-abort',
  toolName: 'shell_abort',
  status: 'completed',
  input: JSON.stringify({ commandId: 'cmd_1' }),
})

export const errorTool = toolCall({
  id: 'tool-error',
  toolName: 'shell_exec',
  status: 'error',
  input: JSON.stringify({ script: 'false', description: 'Remove the stale lock file' }),
  output: [{ type: 'text', text: 'exit 1\npermission denied: /tmp/locked\n' }],
})

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
    statusTool as Block,
    writeTool as Block,
    yieldTool as Block,
    abortTool as Block,
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
