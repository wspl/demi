// The README's picture: a scene that plays one conversation whole, from its
// first message to its second answer, on the computer and the phone at once.
// `readme-picture.ts` records it frame by frame on one stopped clock.
import { reactive } from 'vue'
import type { TerminalRecord } from '@demicodes/web-ui/agent/terminals'
import type { ReplyAsking, ShownSideQuestion } from '@demicodes/web-ui/agent/comments/asks'
import { anchorOfText } from '@demicodes/web-ui/agent/comments/anchors'
import { heldTranscript } from '@demicodes/web-ui/transport/protocol'
import type { WorkspaceView } from '@demicodes/web-ui/workspaces/workspaces'
import { askAnswer, askMessage, askStep } from '@demicodes/web-gallery/fixtures/asks'
import type { GalleryScreen } from '@demicodes/web-gallery/fixtures/live-mobile'
import { gallerySubagents } from '@demicodes/web-gallery/fixtures/subagents'
import { press, shown, shownText } from './dom'
import harborInner from './assets/harbor-inner.webp'
import harborOuter from './assets/harbor-outer.webp'
import { MACBOOK_DEVICE, WINDOWS_PC_DEVICE, ALL_DEVICES, type Scene } from './scenes'
import type { ShellStep } from './conversation'
import type { ReadmeCues } from './readme-page'

export interface ReadmeScene extends Scene {
  /** The account's workspaces, which the sidebar's switcher shows, and the one shown. */
  workspaces: { list: readonly WorkspaceView[]; current: string }
  /** The conversation's messages saved for later, newest first. */
  saved: readonly string[]
  /** What the simulators show, and the 3D body a hinged foldable wears. */
  screen: GalleryScreen
}

/** The path the gallery's files name the body by, which `DemoScene` reads from `DUO_BODY_URL`. */
const DUO_BODY_PATH = '/Applications/Xcode.app/iPhone Duo.usdz'

/**
 * Each display's picture, in its own pixels: Harbor, a small SwiftUI app,
 * running in the iPhone Duo simulator, captured with `demi mobile screenshot`.
 */
const SCREENS = { outer: picture(harborOuter), inner: picture(harborInner) }

function picture(url: string): HTMLImageElement {
  const image = new Image()
  image.src = url
  return image
}

/** Harbor's orders: one column on the cover display, a grid on the open one. */
const HARBOR_SCREEN: GalleryScreen = {
  body: DUO_BODY_PATH,
  paint(context, { width, height }) {
    const image = width > height ? SCREENS.inner : SCREENS.outer
    if (image.complete)
      context.drawImage(image, 0, 0, width, height)
  },
}

/** What the scene adds to the conversation as it plays: its side questions and the suggested replies under its last answer. */
export const readmeExtras = reactive<{
  asking: ReplyAsking | undefined
  suggested: { blockId: string; replies: readonly string[] } | null
}>({ asking: undefined, suggested: null })

const HARBOR = '/Users/alex/Projects/harbor'

const SIMULATOR_LOG: TerminalRecord = {
  id: 'svc-log',
  title: 'Stream Harbor’s log from iPhone Duo',
  script: 'demi mobile log m1 --app app.harbor.ios --follow',
  phase: 'running',
  startedAt: new Date(Date.now() - 30 * 60_000).toISOString(),
  output: 'OrderList: 2 columns, width 760\n',
}

/** The first answer, whose words the user comments on and asks about. */
const LAYOUT_REPLY = `The open display now shows the orders in two columns; the cover keeps one.

- **Layout**: \`OrderList\` turns into an adaptive grid wider than 600 points.
- **Tests**: 42 UI tests pass on both displays, and the reviewer found nothing.`

const COMMENT = { quote: 'adaptive grid', text: 'Cards with long notes get cut off in the grid. Let them grow.' }
const ASK = { quote: 'wider than 600 points', question: 'Why 600?', title: 'Why the grid starts at 600 points', answer: 'The cover display is 466 points wide and the open one 951, so 600 keeps one column on the cover and gives the open display two.' }

/**
 * Each turn's commands, with how long of each the scene plays: a long build
 * shows its whole length yet passes at a glance, and the agent goes on at
 * once, as the picture is watched in a few seconds.
 */
const FIRST_TURN: readonly (readonly [ShellStep, number])[] = [
  [{ description: 'Open iPhone Duo', script: 'demi mobile open "iPhone Duo"', ms: 2200 }, 250],
  [{ description: 'Build and install Harbor', script: 'xcodebuild -scheme Harbor -destination "name=iPhone Duo" build && demi mobile install m1 build/Harbor.app', output: '** BUILD SUCCEEDED **\n', ms: 88_000 }, 500],
  [{ description: 'Unfold it and look at the orders', script: 'demi mobile fold m1 open && demi mobile screenshot m1 | demi file view', ms: 1800 }, 700],
  [{ description: 'Find the order list’s layout', script: 'rg -n "LazyVStack" Harbor/Orders', output: 'Harbor/Orders/OrderList.swift:18:        LazyVStack(spacing: 12) {\n', ms: 400 }, 200],
  [{
    description: 'Lay the orders out in a grid when wide',
    script: 'demi file edit Harbor/Orders/OrderList.swift Harbor/Orders/OrderCard.swift',
    files: [{ path: 'Harbor/Orders/OrderList.swift', added: 14, removed: 5 }, { path: 'Harbor/Orders/OrderCard.swift', added: 3, removed: 1 }],
    ms: 1200,
  }, 300],
  [{ description: 'Keep the reading place through a fold', script: 'demi file edit Harbor/Orders/OrderList.swift', files: [{ path: 'Harbor/Orders/OrderList.swift', added: 9, removed: 0 }], ms: 900 }, 250],
  [{ description: 'Run the UI tests on both displays', script: 'xcodebuild test -scheme HarborUITests -destination "name=iPhone Duo"', output: 'Executed 42 tests, with 0 failures\n', ms: 214_000 }, 500],
  [{ description: 'Ask a reviewer', script: 'demi agent spawn --profile reviewer --description "Review the order grid" < brief.md', output: 'No issues found.\n', ms: 96_000 }, 400],
]

const SECOND_TURN: readonly (readonly [ShellStep, number])[] = [
  [{ description: 'Let order cards grow with their notes', script: 'demi file edit Harbor/Orders/OrderCard.swift', files: [{ path: 'Harbor/Orders/OrderCard.swift', added: 6, removed: 2 }], ms: 900 }, 400],
  [{ description: 'Check a long note on the open display', script: 'demi mobile tap m1 text="Towels, set of 4" && demi mobile screenshot m1 | demi file view', ms: 2600 }, 1300],
]

export const README_SCENES: Record<string, ReadmeScene> = {
  code: {
    title: 'Two-column orders on iPhone Duo',
    cwd: HARBOR,
    primary: MACBOOK_DEVICE,
    devices: ALL_DEVICES,
    workspaces: {
      list: [
        { id: 'personal', name: 'Personal', icon: 'user', color: 'blue' },
        { id: 'work', name: 'Acme', icon: 'briefcase', color: 'purple' },
        { id: 'lab', name: 'Home Lab', icon: 'house', color: 'green' },
      ],
      current: 'work',
    },
    services: [SIMULATOR_LOG],
    workflow: {
      choice: { type: 'workflow', id: 'review' },
      list: { state: 'read', workflows: [{ id: 'review', name: 'Review before merge', path: `${HARBOR}/.demi/workflows/review.yaml`, unavailable: null, profiles: ['reviewer'], userProfiles: true }] },
    },
    saved: ['Run the UI tests on the Android foldable too', 'Write the release notes for 2.3'],
    screen: HARBOR_SCREEN,
    async play({ conversation: c, work, devices, commenting }) {
      devices.attached = [WINDOWS_PC_DEVICE]
      const desktop = document.documentElement.clientWidth > 600
      const cues: ReadmeCues = (window.demiReadme = { marks: {} })
      // `demi mobile open` shows the device in the panel, as the product's
      // does; the panel stands open from the start, so the conversation never
      // narrows on camera.
      if (desktop) {
        await work.mobile.agentOpens('iPhone Duo')
        work.open.value = true
        press(await shownText(c, '[data-work-window] [role="tab"]', 'Duo'))
      }
      // The recorder starts the scene on every screen at once.
      await new Promise<void>((resolve) => (cues.cue = resolve))
      c.asked('The order list feels cramped on iPhone Duo when it’s unfolded. Use two columns on the open display, check it in the simulator, and get it reviewed.', 0)
      for (const [step, playMs] of FIRST_TURN) {
        if (step.script.startsWith('demi mobile fold') && desktop)
          void work.mobile.agentFolds()
        await c.run(step, playMs)
      }
      await c.say(LAYOUT_REPLY)
      c.done()
      const reply = c.state.blocks.findLast((block) => block.type === 'text')!.id
      const text = await shown<HTMLElement>(c, `[data-reply-text="${reply}"] .markdown-body`)
      const commentAt = anchorOfText(text, COMMENT.quote)!
      const askAt = anchorOfText(text, ASK.quote)!
      const comment = { id: 'comment-1', blockId: reply, ...commentAt, at: commentAt.start, quote: COMMENT.quote }
      // The user comments on words of the reply on the computer, then asks of
      // others; the side question runs and answers on its own.
      await c.wait(250)
      if (desktop)
        commenting.add({ ...comment, content: { text: COMMENT.text, fileIds: [] } })
      await c.wait(350)
      for (const blocks of [1, 2, 3]) {
        readmeExtras.asking = asking(sideQuestion(reply, askAt, blocks))
        await c.wait(300)
      }
      if (desktop)
        commenting.remove(comment.id)
      c.commented('Otherwise it looks good.', [{ ...comment, content: [{ type: 'text', text: COMMENT.text }] }])
      for (const [step, playMs] of SECOND_TURN) {
        if (step.script.startsWith('demi mobile tap'))
          cues.marks.check = performance.now()
        await c.run(step, playMs)
      }
      await c.say('Cards now grow with their notes on both displays. Shall I open a pull request?')
      c.done()
      const last = c.state.blocks.findLast((block) => block.type === 'text')!.id
      readmeExtras.suggested = { blockId: last, replies: ['Open a pull request'] }
      cues.marks.done = performance.now()
    },
  },
}

/** The side question asked of `words` in the reply `blockId`, with the first `shown` of its blocks: answered with all three. */
function sideQuestion(blockId: string, words: { start: number; end: number }, shown: number): ShownSideQuestion {
  const id = 'ask-1'
  const blocks = [
    askMessage(id, 0, ASK.quote, [{ type: 'text', text: ASK.question }]),
    askStep(id, 0, 0, { description: 'Read both displays’ sizes', script: 'demi mobile list | grep Duo', output: 'iPhone Duo · cover 466×678 pt · open 951×669 pt\n' }),
    askAnswer(id, 0, ASK.answer),
  ].slice(0, shown)
  return {
    id,
    blockId,
    ...words,
    quote: ASK.quote,
    question: [{ type: 'text', text: ASK.question }],
    title: ASK.title,
    folded: false,
    askedAt: new Date().toISOString(),
    history: heldTranscript(blocks.length, [{ start: 0, blocks }]),
    failures: {},
    working: shown < 3,
    activity: { pendingCalls: [], waitingSince: null, actionSent: false },
  } as ShownSideQuestion
}

/** The side questions as the transcript reads them; the scene takes no new ones. */
function asking(question: ShownSideQuestion): ReplyAsking {
  const nothing = () => {}
  return { questions: [question], ask: nothing, followUp: nothing, retry: nothing, stop: nothing, remove: nothing, fold: nothing, read: nothing } as unknown as ReplyAsking
}

/** The subagents the scene shows working beside its conversation. */
export function readmeSubagents(parent: string) {
  const names = ['Review the order grid', 'Check the fold on every size']
  return gallerySubagents().slice(0, 2).map((agent, i) => ({ ...agent, parent, name: names[i]!, onPath: true }))
}
