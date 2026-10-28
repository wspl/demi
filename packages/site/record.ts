// `bun run record` in this package: the demos' recordings in `public/demos/`.
//
// Each demo is Demi's own app (`src/demo`, from `web-ui` and the gallery's
// fixtures) playing a scripted conversation. Vite builds that app on its
// own, a browser plays each demo on each screen while `record/recorder.ts`
// writes down the page, and the site's frames play the recordings back
// (`src/player`), so the site ships none of the app's script. The app's
// styles, fonts and images go with the recordings, under `demos/assets/`.
import { cp, mkdir, readdir, rename, rm, writeFile } from 'node:fs/promises'
import { extname, join, resolve } from 'node:path'
import { chromium } from 'playwright'
import type { Recording } from './src/player/recording'
import './record/recorder-types'
import { build } from 'vite'

const root = import.meta.dir

// The demo list is the site's own; its `@/` imports resolve here as Vite resolves them.
Bun.plugin({
  name: 'site alias',
  setup(builder) {
    builder.onResolve({ filter: /^@\// }, (args) => ({ path: resolve(root, 'src', args.path.slice(2)) }))
  },
})
const { DEMOS } = await import('./src/features')
const app = resolve(root, 'dist-record')
const out = resolve(root, 'public/demos')
// The recordings are written here and take the place of `out` once all are
// done, so a site that runs meanwhile keeps playing the old ones, styles
// and all.
const staging = resolve(root, 'dist-recording')
/** The screens a frame stands for, in CSS pixels, as `DemoStage.vue` draws them. */
const FORMS = { desktop: { width: 1024, height: 614 }, phone: { width: 393, height: 852 } } as const
/** How long past its tour time a demo plays, so its last change is in. */
const TAIL_MS = 500
/** How long a scene may set up before its opening shot. */
const OPENING_MS = 5000

await rm(staging, { recursive: true, force: true })
await build({
  root,
  mode: 'record',
  base: '/demos/',
  logLevel: 'warn',
  build: { outDir: app, emptyOutDir: true },
})
const recorder = await Bun.build({ entrypoints: [resolve(root, 'record/recorder.ts')], target: 'browser' })
const recorderScript = await recorder.outputs[0].text()

const server = Bun.serve({
  port: 0,
  async fetch(request) {
    const path = new URL(request.url).pathname.replace(/^\/demos\//, '/')
    const file = Bun.file(join(app, path === '/' ? 'record.html' : path))
    return (await file.exists()) ? new Response(file) : new Response('Not found', { status: 404 })
  },
})

await mkdir(staging, { recursive: true })
const browser = await chromium.launch()
try {
  const only = process.argv[2]
  for (const demo of DEMOS.filter((each) => !only || each.id === only)) {
    for (const [form, viewport] of Object.entries(FORMS)) {
      const page = await browser.newPage({ viewport, deviceScaleFactor: 2 })
      await page.addInitScript(recorderScript)
      await page.goto(`http://127.0.0.1:${server.port}/demos/record.html?demo=${demo.id}&form=${form}`)
      // The tour's time runs from the opening shot; a scene without one opens as it loads.
      await page.waitForFunction(() => window.recording?.events.some((event) => event[1] === 'message' && (event[2] as { cut?: boolean } | null)?.cut === true), undefined, { timeout: OPENING_MS }).catch(() => {})
      await page.waitForTimeout(demo.seconds * 1000 + TAIL_MS)
      const recording: Recording = await page.evaluate(() => window.recording!)
      await writeFile(join(staging, `${demo.id}-${form}.json`), JSON.stringify(recording))
      console.log(`Recorded ${demo.id} on the ${form}`)
      await page.close()
    }
  }
} finally {
  await browser.close()
  server.stop()
}

// The app's styles, fonts and images, and the files of `public/` it shows,
// such as the app's icon; its script and pages stay behind.
for (const entry of await readdir(app, { withFileTypes: true })) {
  if (entry.isFile() && extname(entry.name) !== '.html')
    await cp(join(app, entry.name), join(staging, entry.name))
}
await mkdir(join(staging, 'assets'), { recursive: true })
for (const name of await readdir(join(app, 'assets'))) {
  if (extname(name) !== '.js' && !name.endsWith('.js.map'))
    await cp(join(app, 'assets', name), join(staging, 'assets', name))
}
await rm(app, { recursive: true, force: true })

// A whole recording replaces the old one; one demo's joins the others, whose
// recordings keep the styles they name.
if (process.argv[2]) {
  await cp(staging, out, { recursive: true })
  await rm(staging, { recursive: true, force: true })
} else {
  await rm(out, { recursive: true, force: true })
  await rename(staging, out)
}
