import type { Block, ChangedFile, EditedFile, PathChange } from '@demicodes/protocol'
import type { ToolCallBlock } from '@demicodes/web-ui/agent/block-types'
import { binaryFile, demoModel, inRun, listedFile, listedFiles, ownLists, shellView, uncopiedFile, withEnd, withLists } from './blocks'
import { editCopies } from './blobs'
import { ago } from './time'
import { SLOT_ROOT, WORKSPACE_ROOT } from './workspace'

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
const CHECK = 'import re, sys\n\npage = open(\'src/page.ts\').read()\nsys.exit(0 if re.search(r\'<Header />\', page) else 1)\n'

/**
 * A file a call changed, one segment per step between `versions`: of the
 * gallery workspace, or anywhere when `path` is absolute, in which case it
 * is outside the agent's directory, as the backend marks it.
 */
function file(path: string, kind: EditedFile['kind'], versions: readonly string[], counts: { added: number; removed: number }): EditedFile {
  return {
    path: path.startsWith('/') ? path : `${WORKSPACE_ROOT}/${path}`,
    kind,
    ...counts,
    ...(path.startsWith('/') ? { outside: true } : {}),
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
  return withEnd({
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
  })
}

/** The files a shell call's view lists. */
function ownFiles(block: ToolCallBlock): EditedFile[] {
  return block.view?.kind === 'shell' ? block.view.files ?? [] : []
}

/** A file a call removed, or renamed from `from`, as its view lists it. */
function moved(path: string, kind: 'deleted' | 'renamed', from?: string): EditedFile {
  return { path: `${WORKSPACE_ROOT}/${path}`, kind, ...(from ? { from: `${WORKSPACE_ROOT}/${from}` } : {}), added: 0, removed: 0, edits: [] }
}

/** A file listed removed or renamed without an edit. */
function listedMove(file: EditedFile): ChangedFile {
  return { path: file.path, kind: file.kind, ...(file.from ? { from: file.from } : {}), edits: [] }
}

/**
 * The design's example: the user asks to fix the sign-in page. The agent
 * edits login.ts twice and creates form.css in one call, starts the build
 * and ends its turn while it runs; woken by the build's report, it edits
 * login.ts a third time and replies. One request, two files, login.ts edited three times.
 */
export function signInRequestBlocks(prefix: string): Block[] {
  const fix = shell(`${prefix}-fix`, 590_000, 'demi file patch < fix.patch', 'Fix the sign-in check and add the form styles', [
    file('src/auth/login.ts', 'modified', LOGIN.slice(0, 3), { added: 2, removed: 2 }),
    file('src/auth/form.css', 'added', ['', FORM], { added: 8, removed: 0 }),
  ])
  const error = shell(`${prefix}-error`, 500_000, 'demi file edit src/auth/login.ts', 'Throw the old error text', [
    file('src/auth/login.ts', 'modified', LOGIN.slice(2), { added: 1, removed: 1 }),
  ])
  const [login, form] = listedFiles(fix)
  const errorRun = listedFiles(error)
  // login.ts from before the request to after its third edit.
  const request = [{ ...login!, lines: { added: 2, removed: 2 }, edits: [...login!.edits, ...errorRun[0]!.edits] }, form!]
  return [
    user(`${prefix}-user`, 600_000, 'Fix the sign-in page: an empty email must not sign in, and the form fields should fill the width.'),
    ...inRun(fix, withLists(shell(`${prefix}-build`, 580_000, 'bun run build', 'Start the build'), listedFiles(fix))),
    text(`${prefix}-waiting`, 570_000, 'The build is running. I will go on once it ends.'),
    ...inRun(
      { type: 'wakeup', id: `${prefix}-wakeup`, turnId: `${prefix}-wakeup-turn`, createdAt: ago(510_000), model: demoModel, placement: 'new_turn', reports: [{ commandId: `cmd-${prefix}-build`, title: 'Start the build', event: { kind: 'ended', exitCode: 0 }, output: 'built in 4.2s' }] },
      withLists(error, errorRun, request),
    ),
    text(`${prefix}-reply`, 490_000, 'An empty email no longer signs in: it throws the old error text. The form fields now fill the width, and the build passes.'),
  ]
}

/**
 * A request whose calls kept no contents of a binary file, and none of the
 * readme's first edit: the logo is listed without a diff, and the readme,
 * with no original to start All Changes from, opens at its second edit.
 */
export function uncopiedRequestBlocks(): Block[] {
  const logo = shell('uncopied-logo', 390_000, 'cp new-logo.png assets/logo.png', 'Replace the logo', [
    binaryFile({ path: `${WORKSPACE_ROOT}/assets/logo.png`, kind: 'modified', added: 0, removed: 0 }),
    uncopiedFile({ path: `${WORKSPACE_ROOT}/README.md`, kind: 'modified', added: 1, removed: 1 }),
  ])
  const readme = shell('uncopied-readme', 380_000, 'demi file edit README.md', 'Say when the logo changed', [
    file('README.md', 'modified', README, { added: 1, removed: 1 }),
  ])
  // The readme's first original was not kept, so it has no lines to show.
  const [image, firstReadme] = listedFiles(logo)
  const files = [image!, { ...firstReadme!, edits: [...firstReadme!.edits, ...listedFiles(readme)[0]!.edits] }]
  return [
    user('uncopied-user', 400_000, 'Replace the logo and say so in the readme.'),
    ...inRun(logo, withLists(readme, files)),
    text('uncopied-reply', 370_000, 'The logo is replaced and the readme says so.'),
  ]
}

/**
 * A request whose files are listed as they stand after its last call: the
 * new page, written in /tmp and moved over src/page.ts, is src/page.ts with
 * both edits. What the agent wrote on its way shows nowhere, neither in the
 * list nor as a pill: the check script it left in /tmp, outside its
 * directory, and the scratch notes a later call removed
 * (`edit-tracking.md` § Within the working directory).
 */
export function movedRequestBlocks(): Block[] {
  const write = shell('moved-write', 250_000, 'cat > /tmp/page.ts.new <<\'EOF\'\n…\nEOF', 'Write the new page', [
    file('/tmp/page.ts.new', 'added', ['', PAGE[1]!], { added: 8, removed: 0 }),
  ])
  const place = shell('moved-place', 240_000, 'mv /tmp/page.ts.new src/page.ts', 'Put the new page in place', [
    file('src/page.ts', 'modified', PAGE, { added: 6, removed: 1 }),
  ], [{ kind: 'renamed', from: '/tmp/page.ts.new', to: `${WORKSPACE_ROOT}/src/page.ts` }])
  // src/page.ts with both edits: the one made in /tmp, and its move over the old page.
  const [newPage] = listedFiles(place)
  const scratch = listedFile('cmd-moved-write', ownFiles(write)[0]!)
  const page = { ...newPage!, edits: [scratch.edits[0]!, ...newPage!.edits] }
  return [
    user('moved-user', 260_000, 'Give the page the new layout, and clean up after yourself.'),
    ...inRun(
      write,
      place,
      shell('moved-check', 235_000, 'cat > /tmp/check-layout.py <<\'EOF\'\n…\nEOF\npython3 /tmp/check-layout.py', 'Check the new layout', [
        file('/tmp/check-layout.py', 'added', ['', CHECK], { added: 4, removed: 0 }),
      ]),
      shell('moved-notes', 230_000, 'printf … > notes.tmp', 'Note what to check', [
        file('notes.tmp', 'added', ['', NOTES], { added: 2, removed: 0 }),
      ]),
      withLists(shell('moved-clean', 220_000, 'rm notes.tmp', 'Remove the scratch notes', [moved('notes.tmp', 'deleted')], [
        { kind: 'removed', path: `${WORKSPACE_ROOT}/notes.tmp` },
      ]), [page]),
    ),
    text('moved-reply', 210_000, 'The page has the new layout, and the scratch notes are gone.'),
  ]
}

/**
 * A request in two runs that removes and renames files: its first run
 * drafts notes and edits the readme, its second removes the notes, the old
 * guide, which was there before, and renames the setup page. The first
 * run's pills keep the notes it made; the second's show them and the old
 * guide struck through and the setup page under its new name; the request
 * lists the readme, the old guide and the renamed page, not the notes
 * (`edit-tracking.md` § Within the working directory).
 */
export function cleanupRequestBlocks(): Block[] {
  const draft = shell('cleanup-draft', 180_000, 'printf … > docs/draft.md && demi file edit README.md', 'Draft the notes and link them', [
    file('docs/draft.md', 'added', ['', NOTES], { added: 2, removed: 0 }),
    file('README.md', 'modified', README, { added: 1, removed: 1 }),
  ])
  const tidy = shell('cleanup-tidy', 160_000, 'rm docs/draft.md docs/old-guide.md && mv docs/setup.md docs/install.md', 'Remove the drafts and rename the setup page', [
    moved('docs/draft.md', 'deleted'),
    moved('docs/old-guide.md', 'deleted'),
    moved('docs/install.md', 'renamed', 'docs/setup.md'),
  ], [
    { kind: 'removed', path: `${WORKSPACE_ROOT}/docs/draft.md` },
    { kind: 'removed', path: `${WORKSPACE_ROOT}/docs/old-guide.md` },
    { kind: 'renamed', from: `${WORKSPACE_ROOT}/docs/setup.md`, to: `${WORKSPACE_ROOT}/docs/install.md` },
  ])
  const [, readme] = listedFiles(draft)
  const [notes, guide, install] = ownFiles(tidy).map(listedMove)
  return [
    user('cleanup-user', 190_000, 'Tidy the docs: draft notes for the release, then drop what is out of date.'),
    ...inRun(ownLists(draft)),
    text('cleanup-drafted', 170_000, 'Drafted the notes and linked them from the readme. Now the old pages.'),
    ...inRun(withLists(tidy, [notes!, guide!, install!], [readme!, guide!, install!])),
    text('cleanup-reply', 150_000, 'The old guide is gone, the setup page is now install.md, and the draft notes are removed.'),
  ]
}

/** The subagent the parent below spawns, whose own transcript holds its request and its changes. */
export const SUBAGENT = 'gallery-subagent'

/** The subagent's transcript: its spawn's message starts its request, and its call renames the cookie. */
export function subagentBlocks(): Block[] {
  return [
    user('subagent-user', 300_000, 'Rename the sid cookie to session in src/auth/cookie.ts.', 'You are a subagent of the conversation’s agent.'),
    ...inRun(ownLists(shell('subagent-rename', 290_000, 'demi file edit src/auth/cookie.ts', 'Rename the cookie', [
      file('src/auth/cookie.ts', 'modified', COOKIE, { added: 1, removed: 1 }),
    ]))),
    text('subagent-reply', 280_000, 'Renamed the cookie to session.'),
  ]
}

/** The parent's transcript: it spawns the subagent and reads its receipt, and changes no file itself. */
export function subagentParentBlocks(): Block[] {
  return [
    user('parent-user', 310_000, 'Have a subagent rename the session cookie.'),
    ...inRun(shell('parent-spawn', 305_000, 'demi agent spawn --description "Rename the cookie"', 'Start a subagent'), {
      type: 'agent_message',
      id: 'parent-receipt',
      turnId: 'parent-receipt-turn',
      createdAt: ago(275_000),
      model: demoModel,
      message: {
        id: `subagent:${SUBAGENT}:1`,
        sender: { id: SUBAGENT, number: 1, description: 'Rename the cookie', round: 1 },
        recipientId: 'root',
        timestamp: ago(275_000),
        content: 'Renamed the cookie to session.',
        event: { type: 'completion', outcome: 'completed' },
      },
    }),
    text('parent-reply', 270_000, 'The subagent renamed the cookie to session in src/auth/cookie.ts.'),
  ]
}

/** A call shown on its own, in a request of its own, as its pills open it. */
export function standaloneRequest(block: Block): Block[] {
  return [user(`${block.id}-request`, 700_000, 'Fix the cookie rename.'), ...inRun(ownLists(block))]
}

/** The child that works in another checkout (`subagents.md` § Working directory). */
export const SLOT_NODE = 'gallery-slot'

/** A `working_directory` block, as the runtime appends one before a child's first request and after each change. */
function workingDirectory(id: string, offsetMs: number, cwd: string): Block {
  return { type: 'context', id, turnId: id, createdAt: ago(offsetMs), model: demoModel, source: 'working_directory', text: `Working directory: ${cwd}`, cwd }
}

/**
 * A child spawned into another checkout: it edits that checkout's
 * src/auth/cookie.ts, links it, then moves into src/auth and links
 * cookie.ts again by its short name. Both links name a file the
 * conversation's directory has too; each reads against the directory the
 * child wrote it in.
 */
export function slotBlocks(): Block[] {
  const edit = shell('slot-edit', 290_000, 'demi file edit src/auth/cookie.ts', 'Rename the cookie in this checkout', [
    file(`${SLOT_ROOT}/src/auth/cookie.ts`, 'modified', COOKIE, { added: 1, removed: 1 }),
  ])
  return [
    workingDirectory('slot-dir', 300_000, SLOT_ROOT),
    user('slot-user', 300_000, 'Rename the sid cookie to session in this checkout, and tell me where.', 'You are a subagent of the conversation’s agent.'),
    ...inRun(ownLists(edit)),
    text('slot-edited', 280_000, 'Renamed the cookie in [src/auth/cookie.ts](src/auth/cookie.ts) of this checkout.'),
    ...inRun(shell('slot-cwd', 270_000, 'demi agent cwd src/auth', 'Move into the auth folder')),
    workingDirectory('slot-dir-auth', 269_000, `${SLOT_ROOT}/src/auth`),
    text('slot-reply', 260_000, 'From here [cookie.ts](cookie.ts) is the same file: the cookie is `session` now.'),
  ]
}
