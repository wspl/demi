// `bun run build` in this package: the site as static files in `dist/`.
//
// The demos' recordings come first (`record.ts`), into `public/`, which Vite
// copies into `dist/` with the rest.
//
// Vite builds the page for the browser, then builds it again for a server
// and renders it once, here, into `dist/index.html`. The words and the hero
// are in the HTML before any script runs, so the page shows at once and
// search engines and link previews read it; the browser's script then takes
// the page over in place. The server build is thrown away: nothing renders
// the page on request. The same server build writes the head's tags and the
// files beside the page (`seo.ts`).
import { readFile, rm, writeFile } from 'node:fs/promises'
import { join, resolve } from 'node:path'
import { pathToFileURL } from 'node:url'
import { chromium } from 'playwright'
import { build } from 'vite'

const root = import.meta.dir

await import('./record')
const dist = resolve(root, 'dist')
const serverDist = resolve(root, 'dist-server')

await build({ root, logLevel: 'warn' })
await build({
  root,
  logLevel: 'warn',
  build: { ssr: 'src/entry-server.ts', outDir: serverDist, emptyOutDir: true },
})

const { SHARE_IMAGE, headHtml, render, textFiles } = await import(pathToFileURL(resolve(serverDist, 'entry-server.js')).href)
const page = resolve(dist, 'index.html')
const template = await readFile(page, 'utf8')
for (const placeholder of ['<!--head-->', '<!--app-->']) {
  if (!template.includes(placeholder))
    throw new Error(`index.html has no ${placeholder} placeholder`)
}
await writeFile(page, template.replace('<!--head-->', headHtml()).replace('<!--app-->', await render()))
for (const [name, text] of Object.entries(textFiles() as Record<string, string>))
  await writeFile(resolve(dist, name), text)
await rm(serverDist, { recursive: true, force: true })
await photographShareImage()
console.log(`Wrote ${page}`)

/**
 * `og.png`: a browser opens the built `og.html` and photographs its card a
 * moment into the first screen's demo, once its fix has been reported and
 * the screens are still. The page is only for this, and leaves the build
 * afterwards.
 */
async function photographShareImage(): Promise<void> {
  const server = Bun.serve({
    port: 0,
    async fetch(request) {
      const file = Bun.file(join(dist, new URL(request.url).pathname))
      return (await file.exists()) ? new Response(file) : new Response('Not found', { status: 404 })
    },
  })
  const browser = await chromium.launch()
  try {
    const tab = await browser.newPage({ viewport: { width: SHARE_IMAGE.width, height: SHARE_IMAGE.height } })
    await tab.goto(new URL('og.html', server.url).href)
    await tab.waitForSelector('.stage.shown')
    await tab.waitForTimeout(SHARE_IMAGE.momentMs)
    // The photograph waits for a still frame: no transition or finite
    // animation running on the page or in the demo's frames, such as a
    // notification sliding away or a reply's words fading in.
    await tab.waitForFunction(() => [document, ...[...document.querySelectorAll('iframe')].map((frame) => frame.contentDocument)]
      .every((page) => !page || page.getAnimations().every((animation) => animation.playState !== 'running' || animation.effect?.getTiming().iterations === Infinity)), undefined, { polling: 50 })
    await tab.locator('.card').screenshot({ path: resolve(dist, SHARE_IMAGE.path) })
  }
  finally {
    await browser.close()
    server.stop()
  }
  await rm(resolve(dist, 'og.html'))
}
