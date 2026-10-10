import type { Block, EditedFile, PathChange } from '@demicodes/protocol'
import type { ToolCallBlock } from '@demicodes/web-ui/agent/block-types'
import { demoModel, shellView, uncopiedFile } from './blocks'
import { editCopies } from './blobs'
import { ago } from './time'
import { WORKSPACE_ROOT } from './workspace'

/**
 * Requests whose calls changed files (`edit-tracking.md` § Delivery to the
 * conversation), for the specimens of the file pills and of the Change
 * view in Conversation mode.
 */

/** login.ts as the request finds it and after each of its three edits. */
const LOGIN = [
  'export function signIn(user: User): boolean {\n  if (!user) return false\n  return check(user.password)\n}\n',
  'export function signIn(user: User): boolean {\n  if (!user) return false\n  return check(user.password.trim())\n}\n',
  'export function signIn(user: User): boolean {\n  if (!user || !user.email) return false\n  return check(user.password.trim())\n}\n',
  'export function signIn(user: User): boolean {\n  if (!user || !user.email) throw new SignInError(\'Wrong email or password.\')\n  return check(user.password.trim())\n}\n',
]
const FORM = '.form {\n  display: grid;\n  gap: 8px;\n}\n\n.form input {\n  width: 100%;\n}\n'
const README = [
  '# Demi\n\nThe logo is a circle.\n',
  '# Demi\n\nThe logo is a circle on a square, since 2.0.\n',
]
const COOKIE = [
  'export const COOKIE = \'sid\'\n',
  'export const COOKIE = \'session\'\n',
]
/** src/page.ts before the request, and the new layout written in /tmp first. */
const PAGE = [
  'export function Page() {\n  return <main>Hello</main>\n}\n',
  'export function Page() {\n  return (\n    <main className="page">\n      <Header />\n      <Content />\n    </main>\n  )\n}\n',
]
const NOTES = 'Check the header on mobile.\nCheck the footer links.\n'

/**
 * A file a call changed, one segment per step between `versions`: of the
 * gallery workspace, or anywhere when `path` is absolute.
 */
function file(path: string, kind: EditedFile['kind'], versions: readonly string[], counts: { added: number; removed: number }): EditedFile {
  return {
    path: path.startsWith('/') ? path : `${WORKSPACE_ROOT}/${path}`,
    kind,
    ...counts,
    edits: versions.slice(1).map((modified, n) => ({ copies: editCopies(versions[n]!, modified) })),
  }
}

function user(id: string, offsetMs: number, text: string, preamble: string | null = null): Block {
  return { type: 'user', id, turnId: `${id}-turn`, createdAt: ago(offsetMs), model: demoModel, content: [{ type: 'text', text }], preamble }
}

function text(id: string, offsetMs: number, body: string): Block {
  return { type: 'text', id, createdAt: ago(offsetMs), model: demoModel, text: body, forkable: true }
}

function shell(
  id: string,
  offsetMs: number,
  script: string,
  description: string,
  files?: EditedFile[],
  pathChanges?: PathChange[],
): ToolCallBlock {
  return {
    type: 'tool_call',
    id,
    createdAt: ago(offsetMs),
    model: demoModel,
    toolUseId: `${id}-use`,
    toolName: 'shell',
    input: JSON.stringify({ script, description }),
    status: 'completed',
    output: [],
    view: shellView({
      commandId: `cmd-${id}`,
      chunks: [{ stream: 'stdout', text: 'done\n' }],
      files,
      ...(pathChanges ? { pathChanges } : {}),
    }),
  }
}

/**
 * The design's example: the user asks to fix the sign-in page. The agent
 * edits login.ts twice and creates form.css in one call, starts the build
 * and ends its turn while it runs; woken by the build's report, it edits
 * login.ts a third time and replies. One request, two files, login.ts edited three times.
 */
export function signInRequestBlocks(prefix: string): Block[] {
  return [
    user(`${prefix}-user`, 600_000, 'Fix the sign-in page: an empty email must not sign in, and the form fields should fill the width.'),
    shell(`${prefix}-fix`, 590_000, 'demi file patch < fix.patch', 'Fix the sign-in check and add the form styles', [
      file('src/auth/login.ts', 'modified', LOGIN.slice(0, 3), { added: 2, removed: 2 }),
      file('src/auth/form.css', 'added', ['', FORM], { added: 8, removed: 0 }),
    ]),
    shell(`${prefix}-build`, 580_000, 'bun run build', 'Start the build'),
    text(`${prefix}-waiting`, 570_000, 'The build is running. I will go on once it ends.'),
    { type: 'wakeup', id: `${prefix}-wakeup`, turnId: `${prefix}-wakeup-turn`, createdAt: ago(510_000), model: demoModel, placement: 'new_turn', reports: [{ commandId: `cmd-${prefix}-build`, title: 'Start the build', event: { kind: 'ended', exitCode: 0 }, output: 'built in 4.2s' }] },
    shell(`${prefix}-error`, 500_000, 'demi file edit src/auth/login.ts', 'Throw the old error text', [
      file('src/auth/login.ts', 'modified', LOGIN.slice(2), { added: 1, removed: 1 }),
    ]),
    text(`${prefix}-reply`, 490_000, 'An empty email no longer signs in: it throws the old error text. The form fields now fill the width, and the build passes.'),
  ]
}

/**
 * A request whose calls kept no contents of a binary file, and none of the
 * readme's first edit: the logo is listed without a diff, and the readme,
 * with no original to start All Changes from, opens at its second edit.
 */
export function uncopiedRequestBlocks(): Block[] {
  return [
    user('uncopied-user', 400_000, 'Replace the logo and say so in the readme.'),
    shell('uncopied-logo', 390_000, 'cp new-logo.png assets/logo.png', 'Replace the logo', [
      uncopiedFile({ path: `${WORKSPACE_ROOT}/assets/logo.png`, kind: 'modified', added: 0, removed: 0 }),
      uncopiedFile({ path: `${WORKSPACE_ROOT}/README.md`, kind: 'modified', added: 1, removed: 1 }),
    ]),
    shell('uncopied-readme', 380_000, 'demi file edit README.md', 'Say when the logo changed', [
      file('README.md', 'modified', README, { added: 1, removed: 1 }),
    ]),
    text('uncopied-reply', 370_000, 'The logo is replaced and the readme says so.'),
  ]
}

/**
 * A request whose files are listed as they stand after its last call: the
 * new page, written in /tmp and moved over src/page.ts, is src/page.ts with
 * both edits; the scratch notes a later call removed are not listed, though
 * their call's own pill still names them.
 */
export function movedRequestBlocks(): Block[] {
  return [
    user('moved-user', 260_000, 'Give the page the new layout, and clean up after yourself.'),
    shell('moved-write', 250_000, 'cat > /tmp/page.ts.new <<\'EOF\'\n…\nEOF', 'Write the new page', [
      file('/tmp/page.ts.new', 'added', ['', PAGE[1]!], { added: 8, removed: 0 }),
    ]),
    shell('moved-place', 240_000, 'mv /tmp/page.ts.new src/page.ts', 'Put the new page in place', [
      file('src/page.ts', 'modified', PAGE, { added: 6, removed: 1 }),
    ], [{ kind: 'renamed', from: '/tmp/page.ts.new', to: `${WORKSPACE_ROOT}/src/page.ts` }]),
    shell('moved-notes', 230_000, 'printf … > notes.tmp', 'Note what to check', [
      file('notes.tmp', 'added', ['', NOTES], { added: 2, removed: 0 }),
    ]),
    shell('moved-clean', 220_000, 'rm notes.tmp', 'Remove the scratch notes', [], [
      { kind: 'removed', path: `${WORKSPACE_ROOT}/notes.tmp` },
    ]),
    text('moved-reply', 210_000, 'The page has the new layout, and the scratch notes are gone.'),
  ]
}

/** The subagent the parent below spawns, whose own transcript holds its request and its changes. */
export const HELPER = 'gallery-helper'

/** The helper's transcript: its spawn's message starts its request, and its call renames the cookie. */
export function helperBlocks(): Block[] {
  return [
    user('helper-user', 300_000, 'Rename the sid cookie to session in src/auth/cookie.ts.', 'You are a helper of the conversation’s agent.'),
    shell('helper-rename', 290_000, 'demi file edit src/auth/cookie.ts', 'Rename the cookie', [
      file('src/auth/cookie.ts', 'modified', COOKIE, { added: 1, removed: 1 }),
    ]),
    text('helper-reply', 280_000, 'Renamed the cookie to session.'),
  ]
}

/** The parent's transcript: it spawns the helper and reads its receipt, and changes no file itself. */
export function helperParentBlocks(): Block[] {
  return [
    user('parent-user', 310_000, 'Have a helper rename the session cookie.'),
    shell('parent-spawn', 305_000, 'demi agent spawn --description "Rename the cookie"', 'Start a helper'),
    {
      type: 'agent_message',
      id: 'parent-receipt',
      turnId: 'parent-receipt-turn',
      createdAt: ago(275_000),
      model: demoModel,
      message: {
        id: `subagent:${HELPER}:1`,
        sender: { id: HELPER, number: 1, description: 'Rename the cookie', round: 1 },
        recipientId: 'root',
        timestamp: ago(275_000),
        content: 'Renamed the cookie to session.',
        event: { type: 'completion', outcome: 'completed' },
      },
    },
    text('parent-reply', 270_000, 'The helper renamed the cookie to session in src/auth/cookie.ts.'),
  ]
}

/** A call shown on its own, in a request of its own, as its pills open it. */
export function standaloneRequest(block: Block): Block[] {
  return [user(`${block.id}-request`, 700_000, 'Fix the cookie rename.'), block]
}
