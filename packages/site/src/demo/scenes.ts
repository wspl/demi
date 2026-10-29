// What each demo's demo shows: the sidebar around the conversation, and
// the script the scene plays on it. Every name, place and number in them is
// invented.
import type { Ref } from 'vue'
import type { Block } from '@demicodes/protocol'
import type { DeviceMark } from '@demicodes/web-ui/devices/mark'
import type { SidebarConversation, SidebarProject } from '@demicodes/web-ui/sidebar/types'
import type { AgentRun, SubagentRecord } from '@demicodes/web-ui/agent/subagents'
import { NO_COST } from '@demicodes/web-ui/agent/context-cost'
import { wholeHistory } from '@demicodes/web-ui/agent/history'
import type { GalleryWork } from '@demicodes/web-gallery/fixtures/work-panel'
import type { TerminalRecord } from '@demicodes/web-ui/agent/terminals'
import type { WorkflowChoice, WorkflowList } from '@demicodes/web-ui/agent/workflow-menu'
import type { ConversationSchedule } from '@demicodes/web-ui/schedules/types'
import type { HostDeviceOption, HostMenuHost } from '@demicodes/web-ui/hosts/types'
import type { DemoId } from '@/features'
import { MODEL, type Conversation, type DemoFile } from './conversation'
import salePost from './assets/sale-post.webp'
import teaser from './assets/teaser.mp4'
import voiceover from './assets/voiceover.mp3'
import cartShot from './assets/cart.webp'
import receipt1 from './assets/receipt-1.webp'
import receipt2 from './assets/receipt-2.webp'
import receipt3 from './assets/receipt-3.webp'
import { press, shown, shownText, until } from './dom'
import type { CameraRect } from './message'
import { begin, showShop } from './shop'
import { harborShows, showHarbor } from './harbor'

/** The screen a frame stands for. */
export type Form = 'desktop' | 'phone'

/** A notification the phone shows over the app. */
export interface Notice {
  title: string
  text: string
}

/** What a scene plays on. */
export interface Stage {
  conversation: Conversation
  work: GalleryWork
  form: Form
  /** The phone's notification, while it shows. */
  notice: Ref<Notice | null>
  /** Whether the desktop's tab is closed, which shows an empty window. */
  closed: Ref<boolean>
  /** Where the conversation runs, which the header's host menu shows. */
  hosts: DemoHosts
  /**
   * Opens the play on the part of the desktop's screen that `target` covers,
   * as `look` names it: the camera cuts there, and the play starts on it,
   * so what the scene set up before shows at once, as the scene's first shot,
   * once it has settled.
   */
  open(target: Element | string | CameraRect | null): Promise<void>
  /**
   * Brings the part of the desktop's screen close that `target` covers, an
   * element, a CSS selector or a rectangle, or shows all of it for `null`,
   * and waits until the camera is there, so what it points at begins once
   * it is in view. The phone shows all of its screen always, and waits as
   * long, to keep in step.
   */
  look(target: Element | string | CameraRect | null): Promise<void>
}

export interface Scene {
  title: string
  cwd: string
  /** The services the conversation runs when the scene opens, which its Service chip counts. */
  services?: readonly TerminalRecord[]
  /** The schedules that wake the conversation, which its Scheduled chip counts. */
  schedules?: readonly ConversationSchedule[]
  /** The conversation's workflow, which its composer's Workflow menu names. */
  workflow?: { choice: WorkflowChoice; list: WorkflowList }
  /** The user's paired devices, which the host menu lists after the Cloud; none by default. */
  devices?: readonly HostDeviceOption[]
  /** The Host the conversation runs on when the scene opens; the Cloud by default. */
  primary?: HostMenuHost
  play(stage: Stage): Promise<void>
}

/** Where the conversation runs: its primary Host, and the devices its agents attached, as the header's host menu shows them. */
export interface DemoHosts {
  primary: HostMenuHost
  attached: HostMenuHost[]
}

/** The Cloud, the user's own sandbox on their server. */
export const CLOUD: HostMenuHost = { id: 'cloud', name: 'Cloud', kind: 'cloud', state: 'online', mark: null }

const MAC: DeviceMark = { logo: 'apple', number: null }

/** The user's Mac and their office PC, paired with their server. */
const ALEX_MBP: HostDeviceOption = { id: 'd-mbp', name: 'alex-mbp', state: 'online', mark: MAC }
const OFFICE_PC: HostDeviceOption = { id: 'd-pc', name: 'office-pc', state: 'online', mark: { logo: 'windows', number: null } }

/** `device` as the host menu names a Host. */
export function hostOf(device: HostDeviceOption): HostMenuHost {
  return { id: device.id, name: device.name, kind: 'device', state: device.state, mark: device.mark }
}

const SHOP_PROJECT = '/Users/alex/Projects/shop'

/** The first screen's conversation, which its notifications name. */
const HERO_TITLE = 'Fix the negative cart total'

export const PROJECTS: SidebarProject[] = [
  {
    id: 'p-shop',
    name: 'shop',
    hostKind: 'device',
    device: { kind: 'paired', name: 'alex-mbp', state: 'online', mark: MAC },
    path: SHOP_PROJECT,
    savedScope: 'conversation',
    savedCount: 0,
  },
  {
    id: 'p-notes',
    name: 'notes',
    hostKind: 'cloud',
    path: '/home/demi/notes',
    savedScope: 'conversation',
    savedCount: 0,
  },
]

function ago(minutes: number): string {
  return new Date(Date.now() - minutes * 60_000).toISOString()
}

/** The sidebar's rows: the scene's conversation, in its project when it has one, then the user's earlier ones. */
export function sidebarRows({ state }: Conversation): SidebarConversation[] {
  return [
    {
      id: state.id,
      title: state.title,
      updatedAt: ago(0),
      status: state.phase === 'running' ? 'active' : 'done',
      projectId: PROJECTS.find((each) => each.path === state.cwd)?.id ?? null,
      pinned: false,
      unread: false,
    },
    { id: 'c-interviews', title: 'Summarize the customer interviews', updatedAt: ago(40), status: 'done', projectId: null, pinned: false, unread: true },
    { id: 'c-laptops', title: 'Compare three laptop models', updatedAt: ago(180), status: 'idle', projectId: null, pinned: false, unread: false },
    { id: 'c-budget', title: 'Q3 budget questions', updatedAt: ago(600), status: 'idle', projectId: null, pinned: false, unread: false },
    { id: 'c-checkout', title: 'Fix the flaky checkout test', updatedAt: ago(90), status: 'done', projectId: 'p-shop', pinned: false, unread: false },
    { id: 'c-release', title: 'Draft the release notes', updatedAt: ago(300), status: 'idle', projectId: 'p-shop', pinned: false, unread: false },
    { id: 'c-reading', title: 'Reading list for June', updatedAt: ago(900), status: 'idle', projectId: 'p-notes', pinned: false, unread: false },
  ]
}

const CART = `Your usual order is in the cart: 4 items, **$77.65** with tax, arriving Thursday.

I stopped at payment. The tab is open in the panel if you want to check out yourself.`

const XLSX = 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet'

/** The receipts the first screen's user sends, as photos from a phone. */
const RECEIPTS: DemoFile[] = [
  { name: 'IMG_2041.webp', mediaType: 'image/webp', url: receipt1, size: 33_990, width: 480, height: 720 },
  { name: 'IMG_2043.webp', mediaType: 'image/webp', url: receipt2, size: 25_670, width: 480, height: 720 },
  { name: 'IMG_2047.webp', mediaType: 'image/webp', url: receipt3, size: 21_680, width: 480, height: 640 },
]

/** The cart the first screen's user photographed, with the bug in it. */
const CART_SHOT: DemoFile = { name: 'Screenshot 2026-06-24.webp', mediaType: 'image/webp', url: cartShot, size: 11_836, width: 480, height: 738 }


/** The first screen's first act: a bug in the user's shop, fixed on their Mac. */
async function fixCart({ conversation: c, form, notice }: Stage): Promise<void> {
  await c.send('A SAVE10 coupon on a small order makes the cart total negative. Can you fix it?', [CART_SHOT])
  await c.shell({
    description: 'Cap the discount at the subtotal',
    script: 'demi file edit src/cart.ts src/cart.test.ts',
    files: [{ path: 'src/cart.ts', added: 3, removed: 1 }, { path: 'src/cart.test.ts', added: 12, removed: 0 }],
    ms: 900,
  })
  await c.shell({ description: 'Run the cart tests', script: 'bun test src/cart.test.ts', output: ' 24 pass\n 0 fail\n', ms: 800 })
  await c.say(`A coupon worth more than the subtotal took the total below zero. The discount now stops at the subtotal, and all 24 cart tests pass:

\`\`\`ts
const discount = Math.min(coupon.amount, subtotal)
return subtotal - discount + tax
\`\`\``, 1200)
  c.done()
  if (form === 'phone') {
    notice.value = { title: HERO_TITLE, text: 'Fixed: the discount stops at the subtotal. All 24 cart tests pass.' }
    c.after(2400, () => (notice.value = null))
  }
}

/** The first screen's second act, in the same conversation: receipts into an expense report, on the same Mac. */
async function fileExpenses({ conversation: c, form, notice }: Stage): Promise<void> {
  await c.send('Thanks! Can you also turn these receipts into my June expense report? The template is in Documents/Expenses.', RECEIPTS)
  await c.shell({
    description: 'Fill the expense template with the receipts',
    script: 'python3 ~/Documents/Expenses/fill.py --month 2026-06 receipts.json',
    output: 'Wrote ~/Documents/Expenses/June expenses.xlsx: 3 items, $365.10\n',
    ms: 1400,
  })
  const report = c.attach({ name: 'June expenses.xlsx', mediaType: XLSX, url: '#', size: 18_342 })
  await c.say(`Done. Your June report is in Documents/Expenses:

| Receipt | Category | Amount |
| --- | --- | --- |
| Harbor Street Cafe | Meals | $14.60 |
| Metro Cab Co. | Travel | $38.50 |
| Lantern Hotel | Lodging | $312.00 |
| **Total** | | **$365.10** |

<attachment id="${report}" />`, 1400)
  c.done()
  if (form === 'phone') {
    notice.value = { title: HERO_TITLE, text: 'Your June report is ready: 3 receipts, $365.10.' }
    c.after(3200, () => (notice.value = null))
  }
}

const BACKUP_TITLE = 'Back up the photo library'

/**
 * The photo library's backup, which closes the desktop's tab while it runs,
 * opens it again at the end, and tells the phone it finished.
 */
async function backUp({ conversation: c, form, notice, closed }: Stage): Promise<void> {
  c.asked('Back up the photo library to the NAS, then check that every file arrived.', 5000)
  if (form === 'desktop')
    c.after(600, () => (closed.value = true))
  await c.shell({
    description: 'Copy the library to the NAS',
    script: 'rsync -a --info=progress2 ~/Pictures/Library/ nas.local:/backup/photos/',
    output: 'sent 48.2G bytes  received 1.1M bytes\n12,408 files transferred\n',
    ms: 1900,
  })
  await c.shell({ description: 'Compare the checksums', script: 'rsync -ac --dry-run ~/Pictures/Library/ nas.local:/backup/photos/ | wc -l', output: '0\n', ms: 400 })
  await c.say('Done. All 12,408 photos are on the NAS, and their checksums match.', 500)
  c.done()
  if (form === 'phone') {
    notice.value = { title: BACKUP_TITLE, text: 'Done. All 12,408 photos are on the NAS, and their checksums match.' }
    c.after(3200, () => (notice.value = null))
  } else {
    // The user opens Demi again: the finished conversation is there.
    c.after(900, () => (closed.value = false))
  }
}

/**
 * Where the desktop's screen, 1024 × 614, shows what a scene brings close
 * before it opens, so the camera is there first.
 */
const SCREEN = {
  /** The page in the work panel, under its tabs and address bar. */
  panelPage: { x: 616, y: 24, width: 408, height: 370 },
  /** The whole screen, the camera close on none of it: a dialog stands in its middle. */
  whole: { x: 0, y: 0, width: 1024, height: 614 },
  /** The work panel, top to bottom: the simulator fills its height. */
  panel: { x: 616, y: 0, width: 408, height: 614 },
  /**
   * The conversation's header and its first steps: the camera's margin falls
   * within the conversation, never on the sidebar's edge.
   */
  steps: { x: 272, y: 0, width: 736, height: 360 },
} satisfies Record<string, CameraRect>

export const SCENES: Record<DemoId, Scene> = {
  watch: {
    title: 'Reorder the office supplies',
    cwd: '/home/demi',
    async play({ conversation: c, work, form, open, look }) {
      c.asked('Reorder our usual office supplies from Paperwell. Stop before you pay.', 20_000)
      const url = 'https://paperwell.example.com/reorder'
      c.ran({ description: 'Open the reorder page', script: `demi browser open t1 ${url} --show`, ms: 1200 })
      // The phone keeps to the conversation; the desktop shows the page beside it.
      await work.agentOpens(url, form === 'desktop')
      const page = form === 'desktop' ? showShop(await shown<HTMLCanvasElement>(c, '[data-work-panel] canvas')) : null
      await open(SCREEN.panelPage)
      if (page)
        begin(page)
      await c.shell({ description: 'Add the usual order to the cart', script: 'demi browser click t1 --role button --name "Add to cart"', ms: 2800 })
      // The camera steps back to the phone as the reply comes.
      void look(null)
      await c.say(CART, 700)
      c.done()
    },
  },
  machines: {
    title: 'Ship the iOS release',
    cwd: '/home/demi',
    devices: [ALEX_MBP, OFFICE_PC],
    async play({ conversation: c, hosts, open }) {
      c.asked('Build the iOS app and run its UI tests on my Mac, then deploy the API from the cloud.', 5000)
      c.ran({ description: 'Attach alex-mbp', script: 'demi host attach alex-mbp', ms: 500 })
      hosts.attached = [hostOf(ALEX_MBP)]
      c.ran({
        description: 'Build the iOS app on alex-mbp',
        script: 'demi host shell --host alex-mbp "xcodebuild -scheme Shop -configuration Release build"',
        output: '** BUILD SUCCEEDED **\n',
        ms: 2400,
      })
      // The steps show open, each naming the machine it runs on.
      press(await shownText(c, '[data-block-id] span', 'Ran 2 commands'))
      await shownText(c, '[data-block-id] span', 'Build the iOS app on alex-mbp')
      await open(SCREEN.steps)
      await c.wait(300)
      await c.shell({
        description: 'Run the UI tests on alex-mbp',
        script: 'demi host shell --host alex-mbp "xcodebuild test -scheme ShopUITests -destination \'platform=iOS Simulator,name=iPhone 17\'"',
        output: 'Executed 48 tests, with 0 failures\n',
        ms: 1100,
      })
      await c.shell({ description: 'Deploy the API from the cloud', script: 'fly deploy --app shop-api', output: 'v42 deployed successfully\n', ms: 1000 })
      await c.say('The iOS build and all 48 UI tests passed on alex-mbp, and the API is live as v42 from the cloud.', 700)
      c.done()
    },
  },
  switch: {
    title: 'Run the checkout tests in Safari',
    cwd: '/home/demi/shop',
    devices: [ALEX_MBP, OFFICE_PC],
    async play({ conversation: c, hosts, open, look }) {
      c.past(
        'Run the checkout tests in Safari.',
        'Safari runs only on macOS, and this conversation runs in the cloud. Move it to your Mac, and I’ll run them there.',
        60_000,
      )
      const menu = await shown(c, '[data-host-menu]')
      await open(SCREEN.steps)
      await c.wait(300)
      // The user moves it, through the header's host menu; the camera steps
      // back first, so the dialog the move asks in stands in the middle.
      press(menu)
      const mac = await shownText(c, '[role="menuitem"], [role="menuitemradio"]', ALEX_MBP.name)
      await look(SCREEN.whole)
      press(mac)
      const confirm = await shownText(c, '[role="button"], button', 'Move')
      await c.wait(500)
      press(confirm)
      await until(c, () => hosts.primary.id === ALEX_MBP.id)
      await c.wait(300)
      c.state.phase = 'running'
      await c.shell({ description: 'Run the checkout tests in Safari', script: 'bunx playwright test checkout --project=webkit', output: '6 passed\n', ms: 900 })
      await c.say('All 6 checkout tests pass in Safari on alex-mbp.', 500)
      c.done()
    },
  },
  phone: {
    title: 'Test the Harbor sign-up',
    cwd: '/home/demi',
    async play({ conversation: c, work, form, open }) {
      harborShows('empty')
      c.asked('Test the sign-up flow of Harbor on the iPhone simulator.', 12_000)
      c.ran({ description: 'Open the iPhone simulator', script: 'demi mobile open "Work iPhone"', ms: 2200 })
      c.ran({ description: 'Install Harbor', script: 'demi mobile install m1 build/Harbor.app', output: 'app.harbor.ios\n', ms: 1800 })
      // The phone keeps to the conversation; the desktop shows the simulator beside it.
      if (form === 'desktop') {
        await work.mobile.agentOpens()
        // `demi mobile open` shows the simulator in the panel, as the product's does.
        work.select('mobile-m1')
        work.open.value = true
        showHarbor(await shown<HTMLCanvasElement>(c, '[data-work-panel] canvas'))
      }
      await open(SCREEN.panel)
      await c.wait(200)
      harborShows('email')
      await c.shell({ description: 'Type the email', script: 'demi mobile type m1 --into id=email "alex@example.com"', ms: 800 })
      harborShows('password')
      await c.shell({ description: 'Type a password', script: 'demi mobile type m1 --into id=password "$HARBOR_PASSWORD"', ms: 700 })
      harborShows('tapped')
      await c.shell({ description: 'Tap Sign Up', script: 'demi mobile tap m1 text="Sign Up"', ms: 400 })
      harborShows('welcome')
      await c.shell({ description: 'Wait for the welcome screen', script: 'demi mobile wait m1 --text "Welcome"', ms: 400 })
      await c.say('Sign-up works: Harbor opens on its welcome screen.', 600)
      c.done()
    },
  },
  media: {
    title: 'Launch the summer sale',
    cwd: '/home/demi',
    async play({ conversation: c, open }) {
      c.asked('Make a launch post, a short teaser and a voiceover for the summer sale.', 6000)
      await shown(c, '[data-block-id]')
      await open(SCREEN.steps)
      await c.wait(200)
      await c.shell({ description: 'Draw the launch post', script: 'demi image generate "Summer sale post for Harbor Goods, 20% off linens" --aspect 1:1 > post.png', ms: 600 })
      await c.shell({ description: 'Make the teaser', script: 'demi video generate "A slow pan over linen on a sunny shelf" --seconds 8 > teaser.mp4', ms: 700 })
      await c.shell({ description: 'Record the voiceover', script: 'demi audio speech "Summer at Harbor Goods: twenty percent off every linen, this week only." > voiceover.mp3', ms: 600 })
      const post = c.attach({ name: 'post.png', mediaType: 'image/webp', url: salePost, size: 33_940, width: 480, height: 480 })
      const clip = c.attach({ name: 'teaser.mp4', mediaType: 'video/mp4', url: teaser, size: 412_000, width: 480, height: 480 })
      const voice = c.attach({ name: 'voiceover.mp3', mediaType: 'audio/mpeg', url: voiceover, size: 96_000 })
      await c.say(`Here’s the launch kit:\n\n<attachment id="${post}" />\n<attachment id="${clip}" />\n\n<attachment id="${voice}" />`, 600)
      c.done()
    },
  },
  hero: {
    title: HERO_TITLE,
    cwd: SHOP_PROJECT,
    services: [{
      id: 't-dev',
      title: 'Start the dev server',
      script: 'bun run dev',
      phase: 'running',
      startedAt: ago(25),
      output: '  VITE ready in 412 ms\n\n  ➜  Local:   http://localhost:5173/\n',
      service: true,
    }],
    schedules: [{
      id: 'sc-nightly',
      name: 'Nightly tests',
      trigger: {
        kind: 'time',
        timing: {
          kind: 'repeat',
          repeat: { every: 1, unit: 'days', start: null },
          tasks: [{ id: 'sc-nightly-1', time: '02:00', days: [], day: { kind: 'date', date: 1 }, month: 1, holidays: null, prompt: 'Run the full test suite and tell me what fails.' }],
        },
      },
      timeZone: 'America/Los_Angeles',
      paused: false,
      nextAt: ago(-9 * 60),
      lastRun: { at: ago(15 * 60), state: 'done', note: null },
    }],
    workflow: {
      choice: { type: 'workflow', id: 'review' },
      list: {
        state: 'read',
        workflows: [{ id: 'review', name: 'Review before merge', path: `${SHOP_PROJECT}/.demi/workflows/review.yaml`, unavailable: null, profiles: ['reviewer'] }],
      },
    },
    async play(stage) {
      const c = stage.conversation
      await fixCart(stage)
      await c.wait(1500)
      await fileExpenses(stage)
      await c.wait(1500)
      await c.send('Make a square post for the summer sale on linens.')
      await c.shell({
        description: 'Draw the summer sale post',
        script: 'demi image generate "Summer sale post for Harbor Goods, 20% off linens" --aspect 1:1 > summer-sale.png',
        ms: 1400,
      })
      const post = c.attach({ name: 'summer-sale.png', mediaType: 'image/webp', url: salePost, size: 33_940, width: 480, height: 480 })
      await c.say(`Here it is, in the shop’s colors:\n\n<attachment id="${post}" />`, 600)
      c.done()
      await c.wait(1500)
      await c.send('How do our prices compare with Northwind, Bluebird and Lumen?')
      await c.shell({ ...START_RESEARCHERS, ms: 700 })
      researching(c)
      await researchersFinish(c, [1500, 700, 600])
      await c.say(PRICING, 1300)
      c.done()
    },
  },
  background: {
    title: BACKUP_TITLE,
    cwd: '/home/demi',
    play: backUp,
  },
}

const PRICING = `| Plan | Ours | Northwind | Bluebird | Lumen |
| --- | --- | --- | --- | --- |
| Starter | $12 | $15 | $10 | Free |
| Team | $29 | $35 | $24 | $30 |
| Business | $59 | $70 | $49 | $65 |

We are mid-priced on every plan. Bluebird undercuts us by about 17%, but caps Team at 10 seats.`

const RESEARCHERS = [
  { company: 'Northwind', found: 'Starter $15, Team $35, Business $70, each a seat a month. Team needs at least 3 seats.' },
  { company: 'Bluebird', found: 'Starter $10, Team $24, Business $49 a seat a month. Team stops at 10 seats.' },
  { company: 'Lumen', found: 'Starter is free for 2 seats; Team $30 and Business $65 a seat a month.' },
]

const START_RESEARCHERS = { description: 'Start three researchers', script: 'demi agent spawn --profile researcher "Northwind pricing" "Bluebird pricing" "Lumen pricing"' }

/** The researchers start, which the Agents chip counts while they work. */
function researching(c: Conversation): void {
  c.state.subagents = RESEARCHERS.map((each, index) => researcher(c.state.id, each, index))
  c.state.waitingSince = Date.now()
}

/** The researchers from `first` on finish one after another, each `after` its turn's milliseconds. */
async function researchersFinish(c: Conversation, after: readonly number[], first = 0): Promise<void> {
  for (const [turn, ms] of after.entries()) {
    const index = first + turn
    await c.wait(ms)
    const agent = c.state.subagents[index]!
    c.state.subagents = c.state.subagents.map((each) => each.id === agent.id ? researcher(c.state.id, RESEARCHERS[index]!, index, each.startedAt) : each)
  }
}

const RESEARCHER_MODEL: AgentRun['model'] = { providerId: 'anthropic', provider: 'Anthropic', model: 'Claude Sonnet', effort: 'medium', fast: false }

/**
 * A researcher the conversation `parent` started: reading its company's pricing page
 * while it works, with what it found once `startedAt` is given, when it ended.
 */
function researcher(parent: string, { company, found }: (typeof RESEARCHERS)[number], index: number, startedAt?: string): SubagentRecord {
  const id = `researcher-${index + 1}`
  const ended = startedAt !== undefined
  const began = startedAt ?? new Date().toISOString()
  const now = new Date().toISOString()
  const fetch: Block = {
    type: 'tool_call',
    id: `${id}-fetch`,
    run: `${id}-fetch`,
    request: `${id}-user`,
    createdAt: began,
    model: MODEL,
    toolUseId: `${id}-fetch-use`,
    toolName: 'shell',
    input: JSON.stringify({ description: `Read ${company}’s pricing page`, script: `demi web fetch https://${company.toLowerCase()}.example.com/pricing` }),
    status: ended ? 'completed' : 'executing',
    output: [],
    writingStartedAt: null,
    endedAt: ended ? now : null,
    view: { kind: 'shell', status: ended ? 'exited' : 'running', ...(ended ? { exitCode: 0 } : {}), commandId: `${index + 30}`, runningMs: 1200, idleMs: 0, viewTruncated: false, chunks: [] },
  }
  const blocks: Block[] = [
    {
      type: 'user',
      id: `${id}-user`,
      request: `${id}-user`,
      turnId: `${id}-turn`,
      createdAt: began,
      model: MODEL,
      content: [{ type: 'text', text: `Read ${company}’s pricing page and list each plan’s price and limits.` }],
      preamble: null,
    },
    fetch,
    ...(ended ? [{ type: 'text', id: `${id}-found`, request: `${id}-user`, createdAt: now, model: MODEL, text: found, forkable: true } as Block] : []),
  ]
  return {
    id,
    number: index + 1,
    onPath: true,
    parent,
    name: `${company} pricing`,
    profile: 'researcher',
    model: RESEARCHER_MODEL,
    round: 1,
    forkedFrom: null,
    cwd: '/home/demi',
    phase: ended ? 'completed' : 'running',
    sessionPhase: ended ? 'idle' : 'running',
    contextUsage: null,
    startedAt: began,
    ...(ended ? { endedAt: now } : {}),
    failures: {},
    pendingCalls: [],
    waitingSince: null,
    cost: NO_COST,
    shownAt: null,
    history: wholeHistory(blocks),
  }
}
