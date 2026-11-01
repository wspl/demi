// `bun run readme-picture` in this package: the README's picture,
// `docs/assets/product.webp`, a moving picture that plays once and rests on
// its last frame.
//
// It plays a scene of `src/demo/readme.ts` in the demos' app on two screens,
// a computer's browser and a phone, and records both frame by frame: each
// page's clock stops, and each frame moves both clocks on by the same step,
// so the screens keep in step however long a frame takes to shoot. The
// pointer turns iPhone Duo's hinge and view as a person drags them. Each pair
// of frames is laid out in a browser window and a phone on the site's cream
// card, and `img2webp` writes them as one lossless animation: a lossy one
// leaves traces of earlier words where the text changes.
//
// It needs Xcode, whose iPhone Duo model the scene's 3D view wears, served
// to the page and never kept, and `img2webp` from libwebp
// (`brew install webp`). A scene that fails, or a screen that does not play
// it to its end, leaves the picture as it was.
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { chromium, type Page } from 'playwright'
import { build } from 'vite'
import { DUO_BODY_URL } from './src/demo/readme-page'

const root = import.meta.dir
const SCENE = 'code'
const OUT = resolve(root, '../../docs/assets/product.webp')
/** iPhone Duo's 3D body in the Xcode `xcode-select` names, where the simulator plugin finds it too. */
const DUO_BODY = join(
  dirname(Bun.spawnSync(['xcode-select', '-p']).stdout.toString().trim()),
  'SharedFrameworks/DeviceKit.framework/Versions/A/PlugIns/CoreDevicePopDeviceKitExtension.devicekitplugin/Contents/Resources/V68.usdz',
)
const FPS = 25
const STEP_MS = 1000 / FPS
/** How long the picture runs past the scene's end, so the hinge and the view come to rest. */
const REST_MS = 600
/** The longest the scene may play, so a scene that never ends stops the recording. */
const LONGEST_MS = 20_000
const DESKTOP = { viewport: { width: 1440, height: 1080 }, deviceScaleFactor: 2 }
const PHONE = { viewport: { width: 393, height: 852 }, deviceScaleFactor: 3 }

const ICON = {
  back: '<path d="m15 18-6-6 6-6"/>',
  forward: '<path d="m9 18 6-6-6-6"/>',
  sidebar: '<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M9 3v18"/>',
  lock: '<rect width="18" height="11" x="3" y="11" rx="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/>',
  share: '<path d="M12 2v13"/><path d="m16 6-4-4-4 4"/><path d="M4 12v8a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-8"/>',
  add: '<path d="M5 12h14"/><path d="M12 5v14"/>',
  tabs: '<rect width="14" height="14" x="8" y="8" rx="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/>',
  signal: '<path d="M2 20h.01"/><path d="M7 20v-4"/><path d="M12 20v-8"/><path d="M17 20V8"/>',
  wifi: '<path d="M12 20h.01"/><path d="M2 8.82a15 15 0 0 1 20 0"/><path d="M5 12.86a10 10 0 0 1 14 0"/><path d="M8.5 16.43a5 5 0 0 1 7 0"/>',
  battery: '<rect width="16" height="10" x="2" y="7" rx="2"/><path d="M22 11v2"/><path d="M6 11v2M10 11v2M14 11v2"/>',
}
const icon = (name: keyof typeof ICON, size: number) => `<svg width="${size}" height="${size}" viewBox="0 0 24 24">${ICON[name]}</svg>`

/**
 * The card: Safari's frame around the computer's screen, and an iPhone at its
 * lower left corner, a little past the window, on the site's cream with its
 * grain.
 */
const CARD = `<!doctype html><meta charset="utf-8"><style>
* { box-sizing: border-box }
body { margin: 0; background: transparent; font-family: -apple-system, BlinkMacSystemFont, sans-serif }
svg { fill: none; stroke: currentColor; stroke-width: 2; stroke-linecap: round; stroke-linejoin: round }
.card { position: relative; width: 1690px; height: 1330px; overflow: hidden; border-radius: 40px; background: #f8f3ec }
.grain { position: absolute; inset: 0; opacity: .07; mix-blend-mode: multiply }
.browser { position: absolute; left: 170px; top: 70px; width: 1440px; overflow: hidden; border: 1px solid #e6dfd6; border-radius: 14px; background: #fcfaf7; box-shadow: 0 2px 4px rgb(60 40 20 / 5%), 0 24px 60px rgb(60 40 20 / 14%) }
.chrome { display: grid; grid-template-columns: auto auto 1fr auto; gap: 18px; align-items: center; height: 52px; padding: 0 18px; border-bottom: 1px solid #e6dfd6; background: #f7f3ee; color: #6f6660 }
.lights { display: flex; gap: 8px }
.lights i { width: 12px; height: 12px; border-radius: 50% }
.lights i:nth-child(1) { background: #ee6a5f } .lights i:nth-child(2) { background: #f5bd4f } .lights i:nth-child(3) { background: #61c454 }
.tools { display: flex; gap: 14px; align-items: center }
.address { display: inline-flex; gap: 7px; align-items: center; justify-content: center; justify-self: center; width: 440px; padding: 6px 0; border-radius: 8px; background: #ece6de; color: #2b2420; font-size: 14px }
.browser img { display: block; width: 1440px }
.phone { position: absolute; left: 80px; top: 605px; width: 310px; border-radius: 16% / 7.4%; background: #1c1918; box-shadow: 0 20px 44px rgb(40 25 15 / 26%), inset 0 0 0 1.5px #4a4442; container-type: inline-size }
.screen { position: relative; margin: 3cqw; overflow: hidden; border-radius: 13cqw; background: #fcfaf7 }
.screen img { display: block; width: 100% }
.island { position: absolute; top: 2.6cqw; left: 50%; width: 31%; height: 8.6cqw; border-radius: 999px; background: #1c1918; transform: translateX(-50%) }
.status { position: absolute; top: 2.6cqw; left: 0; right: 0; display: flex; height: 8.6cqw; align-items: center; justify-content: space-between; padding: 0 9% 0 13%; color: #2b2420; font-size: 4.6cqw }
.status svg { width: 4.4cqw; height: 4.4cqw; stroke-width: 2.6 }
.status svg.battery { width: 5.8cqw; height: 5.8cqw; stroke-width: 2 }
</style>
<div class="card">
  <canvas class="grain" width="1690" height="1330"></canvas>
  <div class="browser">
    <div class="chrome">
      <span class="lights"><i></i><i></i><i></i></span>
      <span class="tools">${icon('back', 20)}${icon('forward', 20)}${icon('sidebar', 18)}</span>
      <span class="address">${icon('lock', 13)}demi.example.com</span>
      <span class="tools">${icon('share', 18)}${icon('add', 20)}${icon('tabs', 18)}</span>
    </div>
    <img id="computer" alt="">
  </div>
  <div class="phone"><div class="screen">
    <img id="phone" alt="">
    <span class="status"><b>9:41</b><span style="display: flex; gap: 4px; align-items: center">${icon('signal', 24)}${icon('wifi', 24)}${icon('battery', 24).replace('<svg', '<svg class="battery"')}</span></span>
    <span class="island"></span>
  </div></div>
</div>
<script>
  // Film grain: gray noise around the middle, the same every run.
  const canvas = document.querySelector('.grain')
  const context = canvas.getContext('2d')
  const pixels = context.createImageData(canvas.width, canvas.height)
  let seed = 1
  const random = () => ((seed = (seed * 16807) % 2147483647) / 2147483647)
  for (let i = 0; i < pixels.data.length; i += 4) {
    const gray = Math.max(0, Math.min(255, 128 + 40 * Math.sqrt(-2 * Math.log(random())) * Math.cos(2 * Math.PI * random())))
    pixels.data.set([gray, gray, gray, 255], i)
  }
  context.putImageData(pixels, 0, 0)
</script>`

if (!(await Bun.file(DUO_BODY).exists()))
  throw new Error(`No iPhone Duo model at ${DUO_BODY}: the picture needs Xcode.`)
if (!Bun.which('img2webp'))
  throw new Error('No img2webp: install libwebp, as `brew install webp`.')

const app = resolve(root, 'dist-readme')
await build({ root, mode: 'record', base: '/', logLevel: 'warn', build: { outDir: app, emptyOutDir: true } })
const server = Bun.serve({
  port: 0,
  async fetch(request) {
    const path = new URL(request.url).pathname
    const file = Bun.file(path === DUO_BODY_URL ? DUO_BODY : join(app, path))
    return (await file.exists()) ? new Response(file) : new Response('Not found', { status: 404 })
  },
})
const frames = await mkdtemp(join(tmpdir(), 'readme-picture-'))
const browser = await chromium.launch()
/** What went wrong on a screen, which stops the recording. */
const errors: string[] = []
try {
  const screens = await Promise.all([open('desktop', DESKTOP), open('phone', PHONE)])
  const [desk, phone] = screens
  await settle(desk, phone)
  const pointer = await hinge(desk)
  const card = await layout()
  await Promise.all(screens.map((page) => page.evaluate(() => window.demiReadme!.cue!())))
  const start = await desk.evaluate(() => performance.now())
  const names: string[] = []
  for (let frame = 0; ; frame++) {
    const at = frame * STEP_MS
    if (errors.length)
      throw new Error(`The scene failed:\n${errors.join('\n')}`)
    const [marks, phoneMarks] = await Promise.all(screens.map((page) => page.evaluate(() => window.demiReadme!.marks)))
    const since = (name: string) => (marks![name] === undefined ? null : at - (marks![name]! - start))
    const done = since('done')
    if (done !== null && done > REST_MS && phoneMarks!.done !== undefined)
      break
    if (at > LONGEST_MS)
      throw new Error(`The scene did not end on both screens in ${LONGEST_MS / 1000} s.`)
    await pointer(since('check'))
    await Promise.all(screens.map((page) => animationsAt(page, at)))
    const [computer, handset] = await Promise.all(screens.map((page) => page.screenshot()))
    const name = join(frames, `${String(frame).padStart(4, '0')}.png`)
    await card(computer!, handset!, name)
    names.push(name)
    await Promise.all(screens.map((page) => page.clock.runFor(STEP_MS)))
  }
  const picture = join(frames, 'product.webp')
  const encode = Bun.spawn(['img2webp', '-loop', '1', '-kmax', '300', '-lossless', '-m', '4', '-d', String(STEP_MS), ...names, '-o', picture], { stdout: 'ignore' })
  if ((await encode.exited) !== 0)
    throw new Error('img2webp failed')
  await Bun.write(OUT, Bun.file(picture))
  console.log(`Wrote ${OUT}: ${names.length} frames, ${((Bun.file(OUT).size) / 1e6).toFixed(1)} MB`)
} finally {
  await browser.close()
  server.stop()
  await rm(frames, { recursive: true, force: true })
  await rm(app, { recursive: true, force: true })
}

/** The scene on `form`'s screen, its clock stopped. */
async function open(form: 'desktop' | 'phone', options: typeof DESKTOP): Promise<Page> {
  const page = await browser.newPage(options)
  page.on('pageerror', (error) => errors.push(`${form}: ${error.message}`))
  await page.clock.install({ time: new Date('2026-06-02T09:41:00') })
  await page.clock.pauseAt(new Date('2026-06-02T09:41:01'))
  await page.goto(`http://127.0.0.1:${server.port}/record.html?readme=${SCENE}&form=${form}`)
  return page
}

/**
 * Lets both screens set up until the scene waits for its cue and iPhone
 * Duo wears Apple's body, in their time and in real time both: the body loads
 * in a worker, whose time the page's clock does not hold.
 */
async function settle(desk: Page, phone: Page): Promise<void> {
  const cued = (page: Page) => page.evaluate(() => !!window.demiReadme?.cue)
  const body = () => desk.evaluate(() => document.querySelector('[data-work-panel] canvas')?.getAttribute('data-body') ?? null)
  for (let i = 0; ; i++) {
    if (errors.length)
      throw new Error(`The scene failed to set up:\n${errors.join('\n')}`)
    if (i === 300)
      throw new Error('The screens did not set up in a minute.')
    await Promise.all([desk.clock.runFor(200), phone.clock.runFor(200)])
    await desk.waitForTimeout(200)
    const shown = await body()
    if (shown === 'stand-in')
      throw new Error(`iPhone Duo's body did not load from ${DUO_BODY}.`)
    if (shown === 'apple' && await cued(desk) && await cued(phone))
      break
  }
  // The body's first frames draw.
  await desk.clock.runFor(500)
  await desk.mouse.move(1, DESKTOP.viewport.height - 1)
}

/**
 * The pointer's drags on iPhone Duo as the agent checks it, by the time since
 * the scene marked `check`: the hinge's slider from flat to about 109°, then
 * the view turned from an empty part of the panel.
 */
async function hinge(desk: Page): Promise<(since: number | null) => Promise<void>> {
  const slider = (await desk.locator('[data-work-panel] [role=slider]').first().boundingBox())!
  const view = (await desk.locator('[data-work-panel] canvas').first().boundingBox())!
  const y = slider.y + slider.height / 2
  const flat = slider.x + slider.width - 6
  const drags = [
    { from: 50, to: 750, x0: flat, x1: slider.x + slider.width * 0.6, y },
    { from: 850, to: 1400, x0: view.x + 12, x1: view.x + 12 - 65, y: view.y + 30 },
  ]
  let held: (typeof drags)[number] | null = null
  return async (since) => {
    const drag = since === null ? undefined : drags.find((each) => since >= each.from && since <= each.to + STEP_MS)
    if (drag && since !== null) {
      if (held !== drag) {
        if (held)
          await desk.mouse.up()
        await desk.mouse.move(drag.x0, drag.y)
        await desk.mouse.down()
        held = drag
      }
      await desk.mouse.move(drag.x0 + (drag.x1 - drag.x0) * eased((since - drag.from) / (drag.to - drag.from)), drag.y)
    } else if (held) {
      await desk.mouse.move(held.x1, held.y)
      await desk.mouse.up()
      held = null
      await desk.mouse.move(1, DESKTOP.viewport.height - 1)
    }
  }
}

function eased(x: number): number {
  const t = Math.min(1, Math.max(0, x))
  return t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2
}

/**
 * Holds every CSS and web animation of `page` at its age on the page's clock,
 * which runs them in real time otherwise, between the frames.
 */
function animationsAt(page: Page, at: number): Promise<void> {
  return page.evaluate((at) => {
    const born = ((window as { readmeBorn?: WeakMap<Animation, number> }).readmeBorn ??= new WeakMap())
    for (const animation of document.getAnimations()) {
      // A scroll-driven animation follows its scroller, not time.
      if (animation.timeline !== document.timeline)
        continue
      if (!born.has(animation))
        born.set(animation, at)
      const age = at - born.get(animation)!
      if (age >= (animation.effect?.getComputedTiming().endTime as number ?? Infinity)) {
        if (animation.playState !== 'finished')
          animation.finish()
        continue
      }
      animation.pause()
      animation.currentTime = age
    }
  }, at)
}

/** A page that lays a frame of each screen out on the card and writes it to a file. */
async function layout(): Promise<(computer: Buffer, handset: Buffer, path: string) => Promise<void>> {
  const page = await browser.newPage({ viewport: { width: 1800, height: 1400 }, deviceScaleFactor: 1 })
  await page.setContent(CARD)
  return async (computer, handset, path) => {
    await page.evaluate(async ([computer, handset]) => {
      const shots = [document.querySelector<HTMLImageElement>('#computer')!, document.querySelector<HTMLImageElement>('#phone')!]
      shots[0]!.src = `data:image/png;base64,${computer}`
      shots[1]!.src = `data:image/png;base64,${handset}`
      await Promise.all(shots.map((shot) => shot.decode()))
    }, [computer.toString('base64'), handset.toString('base64')])
    await page.locator('.card').screenshot({ path, omitBackground: true })
  }
}

