// What each demo's demo shows: the sidebar around the conversation, and
// the script the scene plays on it. Every name, place and number in them is
// invented.
import type { Ref } from 'vue'
import type { DeviceMark } from '@demicodes/web-ui/devices/mark'
import type { SidebarConversation, SidebarProject } from '@demicodes/web-ui/sidebar/types'
import type { GalleryWork } from '@demicodes/web-gallery/fixtures/work-panel'
import type { TerminalRecord } from '@demicodes/web-ui/agent/terminals'
import type { WorkflowChoice, WorkflowList } from '@demicodes/web-ui/agent/workflow-menu'
import type { ConversationSchedule, ScheduleTask } from '@demicodes/web-ui/schedules/types'
import type { ReplyCommenting } from '@demicodes/web-ui/agent/comments/comments'
import type { HostDeviceOption, HostMenuHost } from '@demicodes/web-ui/hosts/types'
import { DEMOS, type DemoId, type Form } from '@/features'
import type { Conversation, ShellStep } from './conversation'
import clip1 from './assets/clip-1.mp4'
import duoFilm from './assets/duo.mp4'
import clip2 from './assets/clip-2.mp4'
import clip3 from './assets/clip-3.mp4'
import music from './assets/music.mp3'
import salesChart from './assets/sales-chart.webp'
import shot1 from './assets/shot-1.webp'
import shot2 from './assets/shot-2.webp'
import shot3 from './assets/shot-3.webp'
import teaser from './assets/teaser.mp4'
import voiceover from './assets/voiceover.mp3'
import { openRow, press, scrollToEnd, shown, shownText, until } from './dom'
import type { CameraRect, DeviceShot, DevicesShot, Piece, PieceRecipe, RuntimePart, ShotDevice, ShotFlow, ShotMachine, StageCard } from './message'
import { cover } from './cover'
import DuoFilm from './DuoFilm.vue'

export type { Form } from '@/features'

/** What a scene plays on. */
export interface Stage {
  conversation: Conversation
  work: GalleryWork
  form: Form
  /** Where the conversation runs, which the header's host menu shows. */
  hosts: DemoHosts
  /** The schedules that wake the conversation, which its Scheduled chip counts. */
  schedules: Ref<readonly ConversationSchedule[]>
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
  /**
   * Shows `shot` beside the app, or the app alone for `null`: the desktop's
   * frame tells the stage, which shows it for both screens.
   */
  shoot(shot: DeviceShot | null): void
  /** Shows `card` at the browser's corner, beside the desktop's app, or takes the card away for `null`. */
  card(card: StageCard | null): void
  /** The comments waiting in this screen's draft, which the user writes on a reply's words. */
  commenting: ReplyCommenting
  /**
   * The user types `text` into the composer, a few letters at a time over
   * `ms`, and sends it: on `on`'s screen alone when it is given, as the user
   * types on one device, while every other screen waits as long and shows the
   * message once sent.
   */
  compose(text: string, ms: number, on?: Form): Promise<void>
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
  /** Whether the desktop shows the sidebar; collapsed, as the user may keep it, when the scene needs only the conversation. */
  sidebar?: boolean
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

/**
 * The user's machines, paired with their server: the Mac the browser runs
 * on, a Windows PC and a home lab server on the same network, and a VPS. Each
 * names the path this page reaches it by.
 */
const MACBOOK: HostDeviceOption = { id: 'd-mbp', name: 'MacBook', state: 'online', mark: MAC, path: 'thisComputer' }
const WINDOWS_PC: HostDeviceOption = { id: 'd-win', name: 'Windows PC', state: 'online', mark: { logo: 'windows', number: null }, path: 'localNetwork' }
const VPS: HostDeviceOption = { id: 'd-vps', name: 'VPS', state: 'online', mark: { logo: 'ubuntu', number: null }, path: 'internet' }
const HOME_LAB: HostDeviceOption = { id: 'd-home-lab', name: 'Home Lab', state: 'online', mark: { logo: 'debian', number: null }, path: 'localNetwork' }

/** `device` as the host menu names a Host. */
export function hostOf(device: HostDeviceOption): HostMenuHost {
  return { id: device.id, name: device.name, kind: 'device', state: device.state, mark: device.mark }
}

const SHOP_PROJECT = '/Users/alex/Projects/shop'

/** The teaser's storyboard, a shot of each of its scenes, and the clip made from each. */
const STORYBOARD = [shot1, shot2, shot3]
const CLIPS = [clip1, clip2, clip3]

/**
 * The first feature's work: each step a command, and the part of the runtime
 * it uses, with the context compacting halfway.
 */
const AGENT_STEPS: readonly ({ part: RuntimePart; step: ShellStep } | 'compact')[] = [
  { part: 'programs', step: { description: 'Find the price math', script: 'rg -n "toFixed" src/pricing', output: 'src/pricing/total.ts:14:  return total.toFixed(1)\n', ms: 350 } },
  { part: 'browser', step: { description: 'Look at the pricing page', script: 'T=$(demi browser tab new) && demi browser open $T http://localhost:5173/pricing && demi browser screenshot $T | demi file view', ms: 450 } },
  { part: 'programs', step: { description: 'Round to cents', script: 'sed -i \'s/toFixed(1)/toFixed(2)/\' src/pricing/total.ts', files: [{ path: 'src/pricing/total.ts', added: 1, removed: 1 }], ms: 300 } },
  { part: 'host', step: { description: 'Test on the Windows PC', script: 'demi host shell --host "Windows PC" "bun test pricing"', output: '24 pass\n', ms: 500 } },
  'compact',
  { part: 'agent', step: { description: 'Ask a reviewer', script: 'demi agent spawn --profile review --description "Review the rounding fix" < brief.md', output: 'No issues found.\n', ms: 500 } },
  { part: 'programs', step: { description: 'Ship it', script: 'git commit -am "Round prices to cents" && git push', output: 'main -> main\n', ms: 350 } },
]

/** A small helper the user asks for in one sentence, and the schedule the agent makes of a few commands for it. */
interface Helper {
  /** The recipe's name on the pieces card. */
  name: string
  /** The pieces it snaps together, in the order the work passes them. */
  pieces: Piece[]
  ask: string
  description: string
  script: string
  reply: string
  schedule: ConversationSchedule
}

/** A schedule's fields the helpers share: on, in the user's time zone, run never yet. */
function scheduled(id: string, name: string, trigger: ConversationSchedule['trigger'], nextInMinutes: number): ConversationSchedule {
  return { id, name, trigger, timeZone: 'America/Los_Angeles', paused: false, nextAt: ago(-nextInMinutes), lastRun: null }
}

/** A repeat's one task, at `time` on `days`. */
function task(id: string, time: string, days: ScheduleTask['days'], prompt: string): ScheduleTask {
  return { id, time, days, day: { kind: 'date', date: 1 }, month: 1, holidays: null, prompt }
}

const HELPERS: readonly Helper[] = [
  {
    name: 'Price watch',
    pieces: ['schedule', 'web', 'notify'],
    ask: 'Tell me when this jacket drops below $120: shop.example.com/jacket',
    description: 'Check the page hourly, notify on a drop',
    script: `demi schedule create --name "Jacket under $120" --watch 'demi web fetch https://shop.example.com/jacket | grep -o "\\$[0-9]*"' --every 1h <<'EOF'\nIf the jacket is under $120, notify me with demi notify.\nEOF`,
    reply: 'I’ll check the price every hour and notify you when it drops.',
    schedule: scheduled('sc-jacket', 'Jacket under $120', { kind: 'change', command: 'demi web fetch https://shop.example.com/jacket | grep -o "\\$[0-9]*"', everyMinutes: 60 }, 60),
  },
  {
    name: 'Morning briefing',
    pieces: ['schedule', 'search', 'write', 'speech'],
    ask: 'Read me a news briefing at 7 every morning.',
    description: 'Each morning: search the news, write, read it aloud',
    script: `demi schedule create --name "Morning briefing" --repeat day --task-at "07:00" <<'EOF'\nSearch today’s news on my topics, write a two-minute briefing, and send it read aloud with demi audio speech.\nEOF`,
    reply: 'Every morning at 7 you’ll get a spoken two-minute briefing.',
    schedule: scheduled('sc-briefing', 'Morning briefing', { kind: 'time', timing: { kind: 'repeat', repeat: { every: 1, unit: 'days', start: null }, tasks: [task('sc-briefing-1', '07:00', [], 'Search today’s news on my topics, write a two-minute briefing, and send it read aloud with demi audio speech.')] } }, 9 * 60),
  },
  {
    name: 'Weekly report',
    pieces: ['schedule', 'git', 'write', 'file'],
    ask: 'Every Friday, turn the week’s commits into a report.',
    description: 'Each Friday: read the git log, write a report',
    script: `demi schedule create --name "Weekly report" --repeat week --task-at "fri 17:00" <<'EOF'\nSummarize this week’s git log as a report for the team and attach it.\nEOF`,
    reply: 'Every Friday at 5 PM, the week’s report will be here.',
    schedule: scheduled('sc-report', 'Weekly report', { kind: 'time', timing: { kind: 'repeat', repeat: { every: 1, unit: 'weeks', start: null }, tasks: [task('sc-report-1', '17:00', ['fri'], 'Summarize this week’s git log as a report for the team and attach it.')] } }, 3 * 24 * 60),
  },
]

/** The user's own open-source app, a file-sync server they run on their VPS and their home homeLab. */
const TIDEPOOL_PROJECT = '/Users/alex/Projects/tidepool'

/** The first screen's conversation, which its notification names. */
const HERO_TITLE = 'Release v1.4 with the Windows fix'
/** How long the first screen's demo plays before it starts over (`features.ts`). */
const HERO_LOOP_MS = DEMOS.find((demo) => demo.id === 'hero')!.seconds * 1000

export const PROJECTS: SidebarProject[] = [
  {
    id: 'p-tidepool',
    name: 'tidepool',
    hostKind: 'device',
    device: { kind: 'paired', name: 'MacBook', state: 'online', mark: MAC },
    path: TIDEPOOL_PROJECT,
    savedScope: 'conversation',
    savedCount: 0,
  },
  {
    id: 'p-shop',
    name: 'shop',
    hostKind: 'device',
    device: { kind: 'paired', name: 'MacBook', state: 'online', mark: MAC },
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

/**
 * Where the desktop's screen, 1024 × 614, shows what a scene brings close
 * before it opens, so the camera is there first.
 */
const SCREEN = {
  /** The whole screen, the camera close on none of it: a dialog stands in its middle. */
  whole: { x: 0, y: 0, width: 1024, height: 614 },
  /**
   * The conversation's header and its first steps: the camera's margin falls
   * within the conversation, never on the sidebar's edge.
   */
  steps: { x: 272, y: 0, width: 736, height: 360 },
} satisfies Record<string, CameraRect>

/**
 * Where the desktop's screen shows a part of a scene with the sidebar
 * collapsed, measured as it lays out: the conversation, between the window's
 * edge and the work panel when it is open; its first steps; its steps further
 * down, above the composer; and the work panel.
 */
function area(part: 'conversation' | 'steps' | 'later' | 'panel'): CameraRect {
  const panel = document.querySelector('[data-work-panel]')?.getBoundingClientRect()
  const right = panel && panel.width > 0 ? panel.left : 1024
  if (part === 'panel' && panel)
    return { x: panel.left, y: 0, width: panel.width, height: 614 }
  const width = right
  if (part === 'steps')
    return { x: 16, y: 0, width: width - 32, height: Math.round((width - 32) * 0.49) }
  if (part === 'later')
    return { x: 6, y: 76, width: width - 12, height: 420 }
  return { x: 0, y: 0, width, height: 614 }
}

export const MACBOOK_HOST = hostOf(MACBOOK)
export const WINDOWS_PC_HOST = hostOf(WINDOWS_PC)
export const ALL_DEVICES: readonly HostDeviceOption[] = [MACBOOK, WINDOWS_PC, HOME_LAB, VPS]

export const SCENES: Record<DemoId, Scene> = {
  agent: {
    title: 'Fix the price rounding',
    sidebar: false,
    cwd: SHOP_PROJECT,
    devices: [WINDOWS_PC],
    async play({ conversation: c, open, look, card }) {
      c.asked('Prices show one decimal on the pricing page. Fix it, test it on the Windows PC, get a review and ship it.', 3000)
      await shown(c, '[data-block-id]')
      await open(area('steps'))
      // Every step is a command in the one shell: the card lights the part it uses.
      let context = 0.46
      for (const [i, step] of AGENT_STEPS.entries()) {
        if (step === 'compact') {
          card({ kind: 'runtime', part: 'compact', context })
          await c.compact('Prices were rounded to one decimal in `total.ts`; the fix and its tests are done on the Mac and the Windows PC.', 168_000)
          context = 0.14
          card({ kind: 'runtime', part: null, context })
          void look(area('later'))
          continue
        }
        context += 0.11
        card({ kind: 'runtime', part: step.part, context })
        // Each step's row opens as it runs, so its command reads in the transcript.
        const run = c.shell(step.step)
        await openRow(c, step.step.description)
        await run
        // Steps that ran gather in a group, which shows open too.
        if ([...document.querySelectorAll('[data-block-id] button[aria-expanded]')].some((button) => button.parentElement?.textContent?.includes('Ran ')))
          await openRow(c, 'Ran ')
        scrollToEnd()
      }
      card({ kind: 'runtime', part: null, context })
      await c.say('Fixed, tested on the Windows PC, reviewed and pushed.', 500)
      c.done()
      scrollToEnd()
    },
  },
  machines: {
    title: 'Weekly sales update',
    sidebar: false,
    cwd: '/home/demi',
    devices: [MACBOOK, WINDOWS_PC, HOME_LAB, VPS],
    async play({ conversation: c, hosts, open, compose, card, schedules }) {
      // One conversation on every device at once (`AnywhereStage.vue`): the
      // browser sends the task, the phone takes it over, the tablet joins
      // it as the browser goes, and the browser comes back to the result.
      // The stage under each device names the machines the conversation has
      // reached, the one at work now lit.
      let shot: DevicesShot = 'browser'
      const machines: ShotDevice[] = []
      let using: ShotDevice | null = null
      const frame = (): void => card({ kind: 'devices', shot, machines: [...machines], using })
      const at = (next: DevicesShot): void => {
        shot = next
        frame()
      }
      /** The work goes to `machine` for `run`, and rests once it is done. */
      const on = async (machine: ShotDevice, run: Promise<void>): Promise<void> => {
        if (!machines.includes(machine))
          machines.push(machine)
        using = machine
        frame()
        await run
        using = null
        frame()
      }
      // The play opens on an empty conversation, and the task arrives as sent:
      // no typing, just its message, then the work.
      frame()
      await open(null)
      await c.wait(900)
      await c.send('Summarize this week’s sales for the team update.')
      // Whichever device asks, the work runs on the machine it needs.
      hosts.attached = [hostOf(VPS)]
      await on('vps', c.shell({ description: 'Pull this week’s orders from the VPS', script: 'demi host shell --host VPS "psql shop -f weekly.sql"', output: '1,284 orders · $48,920\n', ms: 1100 }))
      // The phone comes in and takes over.
      at('phone')
      await c.say('This week: **1,284 orders** and **$48,920** in sales, up 12% on last week. Linens sold best.', 1200)
      c.done()
      await c.wait(300)
      await compose('Add a chart.', 350, 'phone')
      // The tablet comes in as the browser goes: the two work on together.
      at('both')
      hosts.attached = [hostOf(VPS), hostOf(HOME_LAB)]
      await on('home-lab', c.shell({ description: 'Draw the chart on the Home Lab', script: 'demi host shell --host "Home Lab" "python chart.py" > sales.png', ms: 1100 }))
      const chart = c.attach({ name: 'sales.png', mediaType: 'image/webp', url: salesChart, size: 21_000, width: 960, height: 600 })
      await c.say(`<attachment id="${chart}" />`, 300)
      c.done()
      await c.wait(500)
      await compose('Looks good. Post it to #team.', 500, 'tablet')
      hosts.attached = [hostOf(VPS), hostOf(HOME_LAB), hostOf(MACBOOK)]
      await on('macbook', c.shell({ description: 'Post it to #team from the MacBook', script: 'demi host shell --host MacBook "slack-post #team update.md"', ms: 900 }))
      await c.say('Posted to #team.', 400)
      c.done()
      await c.wait(900)
      // Back at the desk: the browser comes back to all of it, and the user
      // makes it a weekly job there.
      at('back')
      await c.wait(1300)
      await compose('Do this every Friday at 5.', 450, 'desktop')
      await on('vps', c.shell({
        description: 'Schedule it on the VPS for Fridays',
        script: `demi schedule create --name "Weekly sales update" --repeat week --task-at "fri 17:00" <<'EOF'\nSummarize this week’s sales with a chart and post it to #team.\nEOF`,
        ms: 700,
      }))
      schedules.value = [...schedules.value, scheduled('sc-sales', 'Weekly sales update', { kind: 'time', timing: { kind: 'repeat', repeat: { every: 1, unit: 'weeks', start: null }, tasks: [task('sc-sales-1', '17:00', ['fri'], 'Summarize this week’s sales with a chart and post it to #team.')] } }, 3 * 24 * 60)]
      await c.say('Done. Every Friday at 5 PM, the update goes out.', 500)
      c.done()
    },
  },
  across: {
    title: 'Fix and ship the Windows-only failures',
    sidebar: false,
    cwd: SHOP_PROJECT,
    primary: hostOf(MACBOOK),
    devices: [MACBOOK, WINDOWS_PC, HOME_LAB, VPS],
    async play({ conversation: c, hosts, open, look, shoot, compose }) {
      c.asked('Run the end-to-end tests on every machine, and fix what fails.', 4000)
      c.ran({ description: 'Attach the machines', script: 'demi host attach "Windows PC" "Home Lab" VPS', ms: 400 })
      hosts.attached = [hostOf(WINDOWS_PC), hostOf(HOME_LAB), hostOf(VPS)]
      await shown(c, '[data-block-id]')
      await open(area('steps'))
      // Every machine stands in the square beside the browser: the
      // conversation's, those the agent works on now, and those that wait.
      const ALL = ['macbook', 'windows-pc', 'home-lab', 'vps'] as const
      let here: ShotDevice = 'macbook'
      let active: ShotDevice[] = []
      const done = new Set<ShotDevice>()
      const lines: Record<ShotDevice, string[]> = { 'macbook': [], 'windows-pc': [], 'home-lab': [], 'vps': [] }
      const square = (flows: ShotFlow[] = []): void => {
        const machine = (device: ShotDevice): ShotMachine => {
          const idle = here !== device && !active.includes(device) && !done.has(device)
          return { device, lines: [...lines[device]], done: done.has(device), here: here === device, idle }
        }
        shoot({ kind: 'close', beside: 'browser', machines: ALL.map(machine), flows })
      }
      square()
      await c.wait(1000)

      // 1. The tests run on all four at once.
      active = [...ALL]
      lines['macbook'].push('~/shop % bun test e2e')
      lines['windows-pc'].push('PS C:\\shop> bun test e2e')
      lines['home-lab'].push('~/shop $ bun test e2e')
      lines['vps'].push('~/shop $ bun test e2e')
      square()
      const tests = c.together([
        { description: 'Test on the MacBook', script: 'bun test e2e', output: '212 pass\n', ms: 1200 },
        { description: 'Test on the Windows PC', script: 'demi host shell --host "Windows PC" "bun test e2e"', output: '209 pass\n3 fail\n', exitCode: 1, ms: 1700 },
        { description: 'Test on the home lab', script: 'demi host shell --host "Home Lab" "bun test e2e"', output: '212 pass\n', ms: 1400 },
        { description: 'Test on the VPS', script: 'demi host shell --host VPS "bun test e2e"', output: '212 pass\n', ms: 1550 },
      ])
      c.after(1200, () => (lines['macbook'].push('212 pass'), square()))
      c.after(1400, () => (lines['home-lab'].push('212 pass'), square()))
      c.after(1550, () => (lines['vps'].push('212 pass'), square()))
      press(await shownText(c, '[data-block-id] span', 'Ran '))
      await tests
      lines['windows-pc'].push('3 fail')
      square()
      await c.wait(500)

      // 2. The failures' traces come from the Windows PC to the MacBook, which fixes them.
      active = ['windows-pc', 'macbook']
      lines['windows-pc'].push('PS C:\\shop> tar c traces')
      lines['macbook'].push('~/shop % tar x traces')
      const traces: ShotFlow = { from: 'windows-pc', to: 'macbook', label: 'traces' }
      square([traces])
      await c.shell({
        description: 'Copy the failures’ traces here',
        script: 'demi host shell --host "Windows PC" "tar c traces" | tar x',
        ms: 2200,
      })
      active = ['macbook']
      lines['macbook'].push('~/shop % bun run build')
      square()
      await c.shell({ description: 'Fix the path join and build', script: 'bun run build', output: 'dist/ built\n', files: [{ path: 'src/paths.ts', added: 3, removed: 1 }], ms: 900 })
      lines['macbook'].push('dist/ built')
      active = []
      square()
      await c.say('Fixed: paths were joined with `/`. Want me to retest on Windows and ship it?', 500)
      c.done()

      // 3. The user heads out: the conversation moves to the VPS, and the MacBook can sleep.
      const menu = await shown(c, '[data-host-menu]')
      press(menu)
      const target = await shownText(c, '[role="menuitem"], [role="menuitemradio"]', VPS.name)
      await look(SCREEN.whole)
      press(target)
      const confirm = await shownText(c, '[role="button"], button', 'Move')
      await c.wait(400)
      press(confirm)
      await until(c, () => hosts.primary.id === VPS.id)
      here = 'vps'
      square()
      await c.wait(600)
      void look(area('steps'))
      await compose('I’m heading out. Retest on Windows and ship it to the home lab.', 600)

      // 4. The agent goes on from the VPS: Windows retests, and the build goes to the home lab.
      active = ['vps', 'windows-pc']
      lines['vps'].push('~/shop $ git pull')
      lines['windows-pc'].push('PS C:\\shop> bun test e2e')
      square()
      await c.shell({ description: 'Retest on the Windows PC', script: 'demi host shell --host "Windows PC" "git pull && bun test e2e"', output: '212 pass\n', ms: 1500 })
      lines['windows-pc'].push('212 pass')
      active = ['vps', 'home-lab']
      lines['vps'].push('~/shop $ tar c dist')
      lines['home-lab'].push('~/shop $ tar x dist')
      const ship: ShotFlow = { from: 'vps', to: 'home-lab', label: 'dist' }
      square([ship])
      await c.shell({
        description: 'Ship the build to the home lab',
        script: 'tar c dist | demi host shell --host "Home Lab" "tar x -C /srv/shop"',
        ms: 2200,
      })
      lines['home-lab'].push('live')
      active = []
      square()
      await c.say('212 tests pass on Windows too, and the fix is live on the home lab.', 500)
      c.done()

      // 5. Every machine's light turns green, one after another.
      for (const device of ['vps', 'home-lab', 'windows-pc', 'macbook'] as const) {
        await c.wait(220)
        done.add(device)
        square()
      }
      await c.wait(800)
    },
  },
  media: {
    title: 'Make the summer sale teaser',
    sidebar: false,
    cwd: '/home/demi/teaser',
    async play({ conversation: c, open, look, card }) {
      // The timeline card lays out the whole production from the start, and
      // fills in track by track; each part the agent makes shows in the
      // conversation as it comes.
      const stage = (done: number, running: boolean): void => card({ kind: 'production', done, running })
      stage(0, false)
      await open(area('steps'))
      await c.wait(500)
      await c.send('Make a 15-second teaser for the summer sale, with a voiceover and music.')
      const attach = (name: string, mediaType: string, url: string, size: number, side?: number): string =>
        c.attach({ name, mediaType, url, size, ...(side ? { width: side, height: side } : {}) })
      const show = (ids: readonly string[]): string => ids.map((id) => `<attachment id="${id}" />`).join('\n')
      const STEPS: readonly { step: ShellStep; reply: () => string }[] = [
        {
          step: { description: 'Draw the storyboard', script: 'demi image generate "Linen on a sunny shelf, a folded towel, lemonade on the table" --count 3 --aspect 1:1', ms: 1200 },
          reply: () => `The storyboard:\n\n${show(STORYBOARD.map((url, n) => attach(`shot-${n + 1}.png`, 'image/webp', url, 31_000, 480)))}`,
        },
        {
          step: { description: 'Record the voiceover and compose the music', script: 'demi audio speech "Summer at Harbor Goods: twenty percent off every linen." > voice.mp3 && demi audio music "Light acoustic summer tune" --seconds 15 > music.mp3', ms: 1100 },
          reply: () => `The voiceover and the music:\n\n${show([attach('voice.mp3', 'audio/mpeg', voiceover, 35_570), attach('music.mp3', 'audio/mpeg', music, 56_468)])}`,
        },
        {
          step: { description: 'Animate each shot', script: 'for f in shot-*.png; do demi video generate --from "$f" --seconds 5 > "${f%.png}.mp4"; done', ms: 1200 },
          reply: () => `A clip from each shot:\n\n${show(CLIPS.map((url, n) => attach(`shot-${n + 1}.mp4`, 'video/mp4', url, 45_000, 480)))}`,
        },
        {
          step: { description: 'Cut the teaser', script: 'ffmpeg -f concat -i clips.txt -i voice.mp3 -i music.mp3 -filter_complex amix=inputs=2 teaser.mp4', ms: 1300 },
          reply: () => `And the teaser, cut together:\n\n${show([attach('teaser.mp4', 'video/mp4', teaser, 174_048, 480)])}`,
        },
      ]
      for (const [index, { step, reply }] of STEPS.entries()) {
        stage(index, true)
        await c.shell(step)
        stage(index + 1, false)
        await c.say(reply(), 300)
        if (index === 0)
          void look(area('conversation'))
      }
      c.done()
    },
  },
  apps: {
    title: 'Check Harbor on iPhone Duo',
    sidebar: false,
    cwd: '/home/demi',
    async play({ conversation: c, work, form, open, look }) {
      c.asked('Sign up for Harbor on iPhone Duo, then check the orders unfolded.', 12_000)
      c.ran({ description: 'Open iPhone Duo', script: 'demi mobile open "iPhone Duo"', ms: 2200 })
      c.ran({ description: 'Install Harbor', script: 'demi mobile install m1 build/Harbor.app', output: 'app.harbor.ios\n', ms: 1800 })
      // The phone keeps to the conversation; the desktop shows the device beside it, in 3D.
      if (form === 'desktop') {
        await work.mobile.agentOpens('iPhone Duo')
        // `demi mobile open` shows the device in the panel, as the product's does.
        work.open.value = true
        press(await shownText(c, '[data-work-panel] [role="tab"]', 'Duo'))
        // The film covers the device's whole view, its hinge's slider included, which it has no part in.
        const view = (await shown<HTMLCanvasElement>(c, '[data-work-panel] canvas.touch-none')).parentElement!.parentElement!
        await open(area('panel'))
        await until(c, () => view.clientHeight > 400)
        cover(view, DuoFilm, { src: duoFilm }, { width: view.clientWidth, height: view.clientHeight })
      } else {
        await open(area('panel'))
      }
      // The steps keep time with the film: closed, the button pressed; half
      // open; an order picked; open flat.
      await c.shell({ description: 'Tap Sign Up', script: 'demi mobile tap m1 text="Sign Up"', ms: 1000 })
      await c.shell({ description: 'Unfold it halfway', script: 'demi mobile fold m1 half-open', ms: 1100 })
      await c.shell({ description: 'Open the towels’ order', script: 'demi mobile tap m1 text="Towels, set of 4"', ms: 1100 })
      await c.shell({ description: 'Open it flat', script: 'demi mobile fold m1 open', ms: 900 })
      void look(area('conversation'))
      await c.say('Signed up, and the orders read well on both the cover and the open display.', 600)
      c.done()
    },
  },
  pieces: {
    title: 'Little helpers',
    cwd: '/home/demi',
    sidebar: false,
    async play({ conversation: c, schedules, open, look, card }) {
      // Each request snaps a few of the same pieces together, which the card
      // shows as they are placed; then the first one's schedule fires, and the
      // work runs along its pieces to a notification.
      const recipes: PieceRecipe[] = []
      const show = (lit: number | null = null): void => card({ kind: 'pieces', recipes: recipes.map((each) => ({ ...each, pieces: [...each.pieces] })), lit })
      show()
      await open(area('conversation'))
      await c.wait(400)
      for (const helper of HELPERS) {
        await c.send(helper.ask)
        const recipe: PieceRecipe = { name: helper.name, pieces: helper.pieces, placed: 0 }
        recipes.push(recipe)
        show()
        const run = c.shell({ description: helper.description, script: helper.script, ms: 250 * helper.pieces.length + 200 })
        for (let placed = 1; placed <= helper.pieces.length; placed++) {
          await c.wait(250)
          recipe.placed = placed
          show()
        }
        await run
        schedules.value = [...schedules.value, helper.schedule]
        await c.say(helper.reply, 300)
        c.done()
        await c.wait(250)
      }
      // A day later: the price watch fires, and its pieces run in turn.
      await c.wait(400)
      const watch = HELPERS[0]!
      await c.fired('The jacket’s page changed.', { id: watch.schedule.id, name: watch.schedule.name })
      void look(area('later'))
      const fetched = c.shell({ description: 'Read the price', script: 'demi web fetch https://shop.example.com/jacket | grep -o "\\$[0-9]*"', output: '$109\n', ms: 700 })
      show(0)
      await c.wait(350)
      show(1)
      await fetched
      show(2)
      await c.shell({ description: 'Tell you', script: 'demi notify "The jacket is $109 now" --url https://shop.example.com/jacket', ms: 400 })
      await c.say('The jacket dropped to **$109**. I sent the link to your phone.', 400)
      c.done()
      show(null)
    },
  },
  hero: {
    title: HERO_TITLE,
    cwd: TIDEPOOL_PROJECT,
    primary: hostOf(MACBOOK),
    devices: [MACBOOK, WINDOWS_PC, VPS, HOME_LAB],
    schedules: [{
      id: 'sc-nightly',
      name: 'Nightly tests',
      trigger: {
        kind: 'time',
        timing: {
          kind: 'repeat',
          repeat: { every: 1, unit: 'days', start: null },
          tasks: [{ id: 'sc-nightly-1', time: '02:00', days: [], day: { kind: 'date', date: 1 }, month: 1, holidays: null, prompt: 'Run the tests on every machine and tell me what fails.' }],
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
        workflows: [{ id: 'review', name: 'Review before merge', path: `${TIDEPOOL_PROJECT}/.demi/workflows/review.yaml`, unavailable: null, profiles: ['reviewer'], userProfiles: true }],
      },
    },
    play: release,
  },
}

/**
 * The first screen's play: one release across every machine the user has.
 *
 * The fix is made and tested on the Mac in the browser; then the stage cuts to
 * each machine as the agent's work reaches it, the Windows PC for the
 * tests, the VPS and the home lab for the rollout, from the user's own Demi
 * server. The desktop's tab closes halfway and the phone carries on to the
 * notification; the tab opens again on the finished conversation, and the
 * stage ends on every machine at once.
 */
async function release({ conversation: c, hosts, open, look, shoot, compose }: Stage): Promise<void> {
  // Each shot holds about as long as the next, so none flashes by and none
  // lingers. At the desk the work goes back and forth between the Mac and the
  // Windows PC, twice; then the VPS, the VPS with the home lab, and the
  // overview.

  // The Mac the conversation runs on stands at the browser's corner from the
  // start, and the camera comes close as it gets to work.
  const mac: ShotMachine = { device: 'macbook', lines: ['~/tidepool %'], done: false }
  const win: ShotMachine = { device: 'windows-pc', lines: [], done: false }
  const desk = (machine: ShotMachine) => shoot({ kind: 'close', beside: 'browser', machines: [{ ...machine, lines: [...machine.lines] }] })
  desk(mac)
  await open(null)
  const began = performance.now()
  await c.wait(300)
  await compose('Fix the Windows path bug from #212, test it on my PC, then ship v1.4 to the VPS.', 1000)
  void look(SCREEN.steps)

  // The Mac finds where the paths are joined.
  mac.lines = ['% rg -n "join" src/paths.rs', 'src/paths.rs:42:  a + "/" + b']
  desk(mac)
  await c.shell({ description: 'Find where sync paths are joined', script: 'rg -n "join" src/paths.rs', output: 'src/paths.rs:42:  a + "/" + b\n', ms: 1100 })
  await c.shell({ description: 'Attach the Windows PC and the VPS', script: 'demi host attach "Windows PC" && demi host attach VPS', ms: 300 })
  hosts.attached = [hostOf(WINDOWS_PC), hostOf(VPS)]

  // The Windows PC shows the bug.
  win.lines = ['PS C:\\tidepool> cargo test paths', 'test paths::join ... FAILED']
  desk(win)
  await c.shell({
    description: 'Reproduce the bug on the Windows PC',
    script: 'demi host shell --host "Windows PC" "cargo test paths"',
    output: 'test paths::join ... FAILED\n',
    ms: 2900,
  })

  // Back on the Mac: the fix, its tests and the push.
  mac.lines.push('% demi file edit src/paths.rs')
  desk(mac)
  await c.shell({
    description: 'Join the sync paths with the platform’s separator',
    script: 'demi file edit src/paths.rs tests/paths.rs',
    files: [{ path: 'src/paths.rs', added: 4, removed: 2 }, { path: 'tests/paths.rs', added: 18, removed: 0 }],
    ms: 900,
  })
  mac.lines.push('% cargo test', 'test result: ok. 212 passed')
  desk(mac)
  await c.shell({ description: 'Run the tests on the MacBook', script: 'cargo test', output: 'test result: ok. 212 passed; 0 failed\n', ms: 900 })
  mac.lines.push('% git push', '✓ pushed to main')
  mac.done = true
  desk(mac)
  await c.shell({ description: 'Push the fix', script: 'git commit -am "Fix path joins on Windows (#212)" && git push', ms: 600 })

  // Back on the Windows PC: the fix passes there too.
  win.lines.push('PS C:\\tidepool> git pull', 'PS C:\\tidepool> cargo test', 'test result: ok. 212 passed')
  win.done = true
  desk(win)
  await c.shell({
    description: 'Run the tests on the Windows PC',
    script: 'demi host shell --host "Windows PC" "git pull && cargo test"',
    output: 'test result: ok. 212 passed; 0 failed\n',
    ms: 2200,
  })
  await c.shell({ description: 'Tag v1.4.0', script: 'git tag v1.4.0 && git push origin v1.4.0', ms: 700 })
  await c.wait(400)

  // Away from the desk: the browser goes down, the phone comes up on the
  // left to lead, and the VPS comes up beside it.
  const vps: ShotMachine = { device: 'vps', lines: ['$ docker compose pull'], done: false }
  const homeLab: ShotMachine = { device: 'home-lab', lines: ['$ psql tidepool'], done: false }
  // The VPS's data flows into the home lab the whole time both are up.
  const data: ShotFlow = { from: 'vps', to: 'home-lab', label: 'tidepool.sql' }
  const away = (...machines: ShotMachine[]) =>
    shoot({ kind: 'close', beside: 'phone', machines: machines.map((each) => ({ ...each, lines: [...each.lines] })), ...(machines.length > 1 && { flows: [data] }) })
  away(vps)
  // Nothing changes on a screen while it comes or goes: the conversation
  // goes on once the phone stands, and the browser's camera steps back out
  // of sight.
  await c.wait(900)
  void look(null)
  const deploy = c.shell({
    description: 'Deploy v1.4.0 on the VPS',
    script: 'demi host shell --host VPS "docker compose pull && docker compose up -d"',
    output: 'Container tidepool-server-1  Started\n',
    ms: 2200,
  })
  await c.wait(900)
  vps.lines.push('$ docker compose up -d')
  away(vps)
  await compose('Put it on the Home Lab too.', 1000)
  await deploy
  vps.lines.push('✓ healthy · 1.4.0')
  vps.done = true
  away(vps)
  await c.shell({ description: 'Attach the Home Lab', script: 'demi host attach "Home Lab"', ms: 1100 })
  hosts.attached = [hostOf(WINDOWS_PC), hostOf(VPS), hostOf(HOME_LAB)]

  // The home lab comes up beside the VPS, takes its data, and updates.
  vps.lines.push('$ pg_dump tidepool')
  vps.done = false
  away(vps, homeLab)
  await c.shell({
    description: 'Copy the database from the VPS to the Home Lab',
    script: 'demi host shell --host VPS "pg_dump tidepool" | demi host shell --host "Home Lab" "psql tidepool"',
    ms: 1300,
  })
  vps.lines.push('✓ sent 48 MB')
  vps.done = true
  homeLab.lines.push('$ tidepool migrate')
  away(vps, homeLab)
  await c.shell({
    description: 'Update the Home Lab',
    script: 'demi host shell --host "Home Lab" "docker compose up -d && docker compose exec server tidepool migrate"',
    output: 'Applied 2 migrations\n',
    ms: 1000,
  })
  homeLab.lines.push('Applied 2 migrations')
  homeLab.done = true
  away(vps, homeLab)

  await c.say(RELEASED, 600)
  c.done()
  // The demo rests on every machine at once, the app small on the left, and
  // clears the stage just before the play starts over, so the browser comes
  // back with its first screen.
  await c.wait(300)
  shoot({ kind: 'all' })
  await c.wait(HERO_LOOP_MS - 600 - (performance.now() - began))
  shoot({ kind: 'clear' })
}

const RELEASED = `v1.4.0 is out. Paths were joined with \`/\`, which broke sync folders on Windows; they now use the platform’s separator.

| Machine | Done |
| --- | --- |
| MacBook | Fixed, 212 tests pass, tagged v1.4.0 |
| Windows PC | 212 tests pass |
| VPS | Deployed, healthy |
| Home Lab | Updated, 2 migrations applied |`
