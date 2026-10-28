// What each demo's demo shows: the sidebar around the conversation, and
// the script the scene plays on it. Every name, place and number in them is
// invented.
import type { Ref } from 'vue'
import type { DeviceMark } from '@demicodes/web-ui/devices/mark'
import type { SidebarConversation, SidebarProject } from '@demicodes/web-ui/sidebar/types'
import type { GalleryWork } from '@demicodes/web-gallery/fixtures/work-panel'
import type { TerminalRecord } from '@demicodes/web-ui/agent/terminals'
import type { WorkflowChoice, WorkflowList } from '@demicodes/web-ui/agent/workflow-menu'
import type { ConversationSchedule } from '@demicodes/web-ui/schedules/types'
import type { HostDeviceOption, HostMenuHost } from '@demicodes/web-ui/hosts/types'
import { DEMOS, type DemoId } from '@/features'
import type { Conversation } from './conversation'
import salePost from './assets/sale-post.webp'
import teaser from './assets/teaser.mp4'
import voiceover from './assets/voiceover.mp3'
import { press, shown, shownText, until } from './dom'
import type { CameraRect, DeviceShot, ShotFlow, ShotMachine } from './message'
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
  /**
   * Shows `shot` beside the app, or the app alone for `null`: the desktop's
   * frame tells the stage, which shows it for both screens.
   */
  shoot(shot: DeviceShot | null): void
  /** The user types `text` into the composer, a few letters at a time over `ms`, and sends it. */
  compose(text: string, ms: number): Promise<void>
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
const ALEX_MBP: HostDeviceOption = { id: 'd-mbp', name: 'MacBook', state: 'online', mark: MAC }
const OFFICE_PC: HostDeviceOption = { id: 'd-pc', name: 'office-pc', state: 'online', mark: { logo: 'windows', number: null } }

/** The first screen's machines: the user's Mac, their Windows PC at home, a VPS and a home lab server. */
const WINDOWS_PC: HostDeviceOption = { id: 'd-win', name: 'Windows PC', state: 'online', mark: { logo: 'windows', number: null } }
const VPS: HostDeviceOption = { id: 'd-vps', name: 'VPS', state: 'online', mark: { logo: 'ubuntu', number: null } }
const HOME_LAB: HostDeviceOption = { id: 'd-home-lab', name: 'Home Lab', state: 'online', mark: { logo: 'debian', number: null } }

/** `device` as the host menu names a Host. */
export function hostOf(device: HostDeviceOption): HostMenuHost {
  return { id: device.id, name: device.name, kind: 'device', state: device.state, mark: device.mark }
}

const SHOP_PROJECT = '/Users/alex/Projects/shop'

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

const CART = `Your usual order is in the cart: 4 items, **$77.65** with tax, arriving Thursday.

I stopped at payment. The tab is open in the panel if you want to check out yourself.`

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
      c.ran({ description: 'Attach MacBook', script: 'demi host attach MacBook', ms: 500 })
      hosts.attached = [hostOf(ALEX_MBP)]
      c.ran({
        description: 'Build the iOS app on the MacBook',
        script: 'demi host shell --host MacBook "xcodebuild -scheme Shop -configuration Release build"',
        output: '** BUILD SUCCEEDED **\n',
        ms: 2400,
      })
      // The steps show open, each naming the machine it runs on.
      press(await shownText(c, '[data-block-id] span', 'Ran 2 commands'))
      await shownText(c, '[data-block-id] span', 'Build the iOS app on the MacBook')
      await open(SCREEN.steps)
      await c.wait(300)
      await c.shell({
        description: 'Run the UI tests on the MacBook',
        script: 'demi host shell --host MacBook "xcodebuild test -scheme ShopUITests -destination \'platform=iOS Simulator,name=iPhone 17\'"',
        output: 'Executed 48 tests, with 0 failures\n',
        ms: 1100,
      })
      await c.shell({ description: 'Deploy the API from the cloud', script: 'fly deploy --app shop-api', output: 'v42 deployed successfully\n', ms: 1000 })
      await c.say('The iOS build and all 48 UI tests passed on the MacBook, and the API is live as v42 from the cloud.', 700)
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
      await c.say('All 6 checkout tests pass in Safari on the MacBook.', 500)
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
    cwd: TIDEPOOL_PROJECT,
    primary: hostOf(ALEX_MBP),
    devices: [ALEX_MBP, WINDOWS_PC, VPS, HOME_LAB],
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
        workflows: [{ id: 'review', name: 'Review before merge', path: `${TIDEPOOL_PROJECT}/.demi/workflows/review.yaml`, unavailable: null, profiles: ['reviewer'] }],
      },
    },
    play: release,
  },
  background: {
    title: BACKUP_TITLE,
    cwd: '/home/demi',
    play: backUp,
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
    shoot({ kind: 'close', beside: 'phone', machines: machines.map((each) => ({ ...each, lines: [...each.lines] })), ...(machines.length > 1 && { flow: data }) })
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
