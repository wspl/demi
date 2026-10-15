import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { existsSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { compile } from '@tailwindcss/node'
import { chromium, type Browser, type Page } from 'playwright'
import type { renderMarkdownDocument as Render } from '../document'

// A Markdown file shown as a document lays out as GitHub lays out the same
// file (`file-previews.md` § Markdown): a README's centered logo at the
// height its tag gives, its badges on one line, GitHub's alerts, a table
// that fits the page, and the images a theme picks. In a real Chromium, with
// the document renderer's sanitizer and the product's stylesheet as Tailwind
// builds it, whose preflight is what made every image a block. Images come
// from a route that draws an SVG of the size its name says. About 4 s: the
// stylesheet's build and the browser's start, shared, and a page per test.

declare global {
  interface Window {
    renderMarkdownDocument: typeof Render
    /** Renders `markdown` into the page's document, at a width of 600 px, and waits for its images. */
    show(markdown: string, theme: 'light' | 'dark'): Promise<string[]>
  }
}

const WIDTH = 600

describe.skipIf(!existsSync(chromium.executablePath()))('in Chromium', () => {
  let browser: Browser
  let script: string
  let css: string
  beforeAll(async () => {
    const at = (path: string) => JSON.stringify(fileURLToPath(new URL(path, import.meta.url)))
    const entry = fileURLToPath(new URL('./page-entry.ts', import.meta.url))
    const build = await Bun.build({
      entrypoints: [entry],
      files: {
        [entry]: [
          `import { renderMarkdownDocument } from ${at('../document.ts')}`,
          `Object.assign(window, { renderMarkdownDocument })`,
        ].join('\n'),
      },
      target: 'browser',
      conditions: ['development'],
    })
    if (!build.success)
      throw new AggregateError(build.logs, 'The page did not build')
    script = await build.outputs[0]!.text()
    // The product's stylesheet as the web app's build makes it, Tailwind's preflight first.
    const styles = fileURLToPath(new URL('../../styles/', import.meta.url))
    const compiler = await compile(`@import 'tailwindcss';\n@import './base.css';`, { base: styles, onDependency: () => {} })
    css = compiler.build([])
    browser = await chromium.launch()
  })
  afterAll(async () => {
    await browser?.close()
  })

  /** A page with an empty document; `system` is the color scheme the system asks for. */
  async function page(system: 'light' | 'dark' = 'light'): Promise<Page> {
    const opened = await browser.newPage({ colorScheme: system })
    await opened.route('https://demi.test/', (route) => route.fulfill({ contentType: 'text/html', body: '<!doctype html><body></body>' }))
    // `https://img.test/<name>-w<width>h<height>.svg`: a picture of that size.
    await opened.route('https://img.test/**', (route) => {
      const [, width, height] = /w(\d+)h(\d+)\.svg/.exec(route.request().url()) ?? []
      route.fulfill({ contentType: 'image/svg+xml', body: `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}"><rect width="100%" height="100%"/></svg>` })
    })
    await opened.goto('https://demi.test/')
    await opened.addStyleTag({ content: css })
    await opened.addScriptTag({ content: script })
    await opened.evaluate((width) => {
      const render = window.renderMarkdownDocument
      const article = document.createElement('article')
      article.className = 'markdown-body markdown-document'
      article.style.width = `${width}px`
      document.body.append(article)
      window.show = async (markdown, theme) => {
        const rendered = render(markdown, { path: '/work/README.md', root: '/work' }, theme)
        article.innerHTML = rendered.html
        // A Host image's address, as the File view gives it: here the route's, by its file's name.
        for (const image of article.querySelectorAll('img[data-host-file]'))
          image.setAttribute('src', `https://img.test/${image.getAttribute('data-host-file')!.split('/').pop()}`)
        await Promise.all([...article.querySelectorAll('img')].map((image) => image.decode().catch(() => {})))
        return rendered.hostFiles
      }
    }, WIDTH)
    return opened
  }

  /** Where each image of the document stands, relative to the document, by its alt text. */
  async function boxes(opened: Page): Promise<Record<string, { left: number; top: number; width: number; height: number; src: string }>> {
    return opened.evaluate(() => {
      const article = document.querySelector('article')!.getBoundingClientRect()
      return Object.fromEntries([...document.querySelectorAll('article img')].map((image) => {
        const box = image.getBoundingClientRect()
        return [image.getAttribute('alt'), { left: box.left - article.left, top: box.top - article.top, width: box.width, height: box.height, src: (image as HTMLImageElement).currentSrc }]
      }))
    })
  }

  test('a README’s header: the logo at its tag’s height and centered, the badges on one line at their own size', async () => {
    const opened = await page()
    await opened.evaluate((markdown) => window.show(markdown, 'light'), [
      '<p align="center">',
      '  <picture>',
      '    <img alt="Demi" src="brand/logo-w300h90.svg" height="72">',
      '  </picture>',
      '</p>',
      '',
      '<p align="center">',
      '  <a href="#license"><img alt="License" src="https://img.test/license-w210h20.svg"></a>',
      '  <a href="https://example.com/r"><img alt="Release" src="https://img.test/release-w102h20.svg"></a>',
      '  <a href="https://example.com/c"><img alt="CI" src="https://img.test/ci-w90h20.svg"></a>',
      '</p>',
    ].join('\n'))
    const { Demi: logo, License: license, Release: release, CI: ci } = await boxes(opened)
    // 72 px high, as wide as its proportions make it, its middle at the document's.
    expect({ width: logo!.width, height: logo!.height }).toEqual({ width: 240, height: 72 })
    expect(Math.abs(logo!.left + logo!.width / 2 - WIDTH / 2)).toBeLessThanOrEqual(1)
    expect([license, release, ci].map((badge) => [badge!.width, badge!.height])).toEqual([[210, 20], [102, 20], [90, 20]])
    expect(new Set([license, release, ci].map((badge) => badge!.top)).size).toBe(1)
    expect(license!.left).toBeLessThan(release!.left)
    expect(release!.left).toBeLessThan(ci!.left)
    await opened.close()
  })

  test('GitHub’s alerts show their kind as a title, and a marker not alone on its line stays a quote', async () => {
    const opened = await page()
    const text = await opened.evaluate(async (markdown) => {
      await window.show(markdown, 'light')
      return document.querySelector('article')!.innerText
    }, [
      '> [!NOTE]',
      '> Demi is in **early** development.',
      '',
      '> [!warning]',
      '> Mind the gap.',
      '',
      '> [!NOTE] Not alone on its line.',
    ].join('\n'))
    expect(text.split('\n').filter(Boolean)).toEqual([
      'Note',
      'Demi is in early development.',
      'Warning',
      'Mind the gap.',
      '[!NOTE] Not alone on its line.',
    ])
    await opened.close()
  })

  test('a wide table fits the document, its cells wrapping, and a small one is as wide as its content', async () => {
    const opened = await page()
    const row = (cell: string) => `| ${Array.from({ length: 5 }, () => cell).join(' | ')} |`
    const widths = await opened.evaluate(async (markdown) => {
      await window.show(markdown, 'light')
      return [...document.querySelectorAll('table')].map((table) => table.getBoundingClientRect().width)
    }, [
      row('Product'),
      row('---'),
      row('Local worktrees, SSH hosts or your cloud VMs'),
      '',
      '| A | B |',
      '| --- | --- |',
      '| 1 | 2 |',
    ].join('\n'))
    expect(widths[0]).toBeLessThanOrEqual(WIDTH)
    expect(widths[1]).toBeLessThan(WIDTH / 2)
    await opened.close()
  })

  const themed = [
    '<picture>',
    '  <source media="(prefers-color-scheme: dark)" srcset="https://img.test/dark-w100h20.svg">',
    '  <source media="(prefers-color-scheme: light)" srcset="https://img.test/light-w100h20.svg">',
    '  <img alt="Logo" src="https://img.test/plain-w100h20.svg">',
    '</picture>',
    '',
    '![Dark only](https://img.test/donly-w50h20.svg#gh-dark-mode-only) ![Light only](https://img.test/lonly-w50h20.svg#gh-light-mode-only)',
  ].join('\n')

  for (const [theme, system] of [['dark', 'light'], ['light', 'dark']] as const) {
    test(`Demi’s ${theme} theme on a ${system} system picks the ${theme} picture and the ${theme}-only image`, async () => {
      const opened = await page(system)
      await opened.evaluate(([markdown, shown]) => window.show(markdown, shown), [themed, theme] as const)
      const images = await boxes(opened)
      expect(images['Logo']?.src).toBe(`https://img.test/${theme}-w100h20.svg`)
      expect(Object.keys(images).sort()).toEqual(['Logo', theme === 'dark' ? 'Dark only' : 'Light only'].sort())
      await opened.close()
    })
  }

  test('a srcset’s Host candidates are named for the File view to address, and its web ones stay', async () => {
    const opened = await page()
    const result = await opened.evaluate(async (markdown) => {
      const hostFiles = await window.show(markdown, 'dark')
      const source = document.querySelector('source')!
      return { hostFiles, srcset: source.getAttribute('srcset'), host: JSON.parse(source.getAttribute('data-host-srcset') ?? 'null') }
    }, '<picture><source media="(prefers-color-scheme: dark)" srcset="brand/dark.svg 1x, https://img.test/dark-w200h40.svg 2x"><img alt="Logo" src="brand/light.svg"></picture>')
    expect(result.srcset).toBeNull()
    expect(result.host).toEqual([
      { path: '/work/brand/dark.svg', descriptor: '1x' },
      { url: 'https://img.test/dark-w200h40.svg', descriptor: '2x' },
    ])
    expect(result.hostFiles.sort()).toEqual(['/work/brand/dark.svg', '/work/brand/light.svg'])
    await opened.close()
  })
})
