// One call of `bun browse` (browse.md § A call): runs the script in its
// scope, prints what it logged and returned, and ends with the report:
// where and why it failed, if it did, with a screenshot, then the footer.
// A browser that stops answering ends the call within seconds, whatever
// the script waits for, and is closed (`hang.ts`).
import { Console } from 'node:console'
import { writeFileSync } from 'node:fs'
import { Writable } from 'node:stream'
import { inspect } from 'node:util'
import type { Page } from 'playwright'
import { expect } from 'playwright/test'
import type { Browser } from './browser'
import { createDemi } from './demi'
import { describeFailure } from './failure'
import { composeFooter, readPageState, type PageState } from './footer'
import type { Keep } from './keep'
import type { Network } from './network'
import type { Request } from './protocol'
import { Script, type ScriptScope } from './script'
import { pngSize, shotPath } from './shots'
import { slotPaths, type Slot } from './slot'
import { webBase, type Shot, type Tool } from './tool'
import { HANG_MS, HangWatch, HUNG } from './hang'
import { OVERRAN, withinLimit } from './limit'

/** What the server lends a call. */
export interface Server {
  slot: Slot
  browser: Browser
  network(): Promise<Network>
  keep: Keep
}

export interface Outcome {
  exit: number
  /**
   * What the server does once the call has answered: stays for the next
   * call; ends with the browser, as `demi.stop` and `demi.down` ask and a
   * browser that hung needs; or leaves the browser to a new server, since
   * a script that ran into the call's time limit still runs, and only the
   * end of the server's process stops it.
   */
  after: 'stay' | 'end' | 'leave'
}

/**
 * Names that make a script need the page: a script that uses neither, such
 * as `demi.down()`, starts no browser for nothing. A helper that needs the
 * page starts the browser itself.
 */
const USES_PAGE = /\b(page|context|cdp)\b/

/** How long each step of the report after the script, such as its screenshot, may wait for the browser. */
const REPORT_MS = 10_000

/**
 * Runs `request`; a browser that leaves a question unanswered for `hangMs`
 * ends it, and the server with it.
 */
export async function runCall(
  server: Server,
  request: Request,
  print: (line: string) => void,
  hangMs = HANG_MS,
): Promise<Outcome> {
  const { slot, browser } = server
  const shots: Shot[] = []
  const releases: (() => Promise<void>)[] = []
  let endServer = false
  const tool: Tool = {
    slot,
    browser,
    network: server.network,
    print,
    env: request.env,
    wrote: (shot) => shots.push(shot),
    release: (release) => releases.push(release),
    endServer: () => {
      endServer = true
    },
  }
  // The page's problems since the call began are those logged after this entry.
  const since = browser.logs.newest
  const script = new Script(request.script, slotPaths(slot).calls)
  const watch = new HangWatch(browser, hangMs)
  try {
    const ending = await runScript(tool, script, request, server.keep, watch)
    await release(releases, print)
    if (ending.kind === 'overran') {
      print(`Failed: the script ran for ${request.limitMs / 1000} s, its time limit (--limit); the tool's server restarts to stop it and keeps the browser`)
    } else if (ending.kind === 'hung') {
      print(`Failed: the browser did not answer for ${hangMs / 1000} s, so the call stopped waiting for the script`)
    } else if (ending.kind === 'failed') {
      const line = script.lineOf(ending.error)
      const where = { line, text: line === null ? '' : script.text(line), file: request.file }
      for (const text of describeFailure(ending.error, where)) {
        print(text)
      }
    }
    const report = ending.kind === 'hung' ? 'hung' : await reportPage(tool, ending.kind !== 'done', watch)
    if (report === 'hung') {
      // Its pages answer nothing, and the script may still wait on it: the
      // browser goes, and the server with it, which ends the script.
      await browser.close({ hung: true })
      endServer = true
      print('Page      the browser did not answer, so the tool closed it; the next call starts a new one')
    }
    for (const line of composeFooter({ shots, page: report === 'hung' ? null : report, problems: browser.logs.problems(since) })) {
      print(line)
    }
    return {
      exit: ending.kind === 'done' && report !== 'hung' ? 0 : 1,
      after: endServer ? 'end' : ending.kind === 'overran' ? 'leave' : 'stay',
    }
  } finally {
    watch.stop()
    script.remove()
  }
}

/** How the script ended. */
type Ending = { kind: 'done' } | { kind: 'failed', error: unknown } | { kind: 'overran' } | { kind: 'hung' }

/** Runs the script until it ends, runs into the call's limit, or the browser stops answering; prints what it returned. */
async function runScript(tool: Tool, script: Script, request: Request, keep: Keep, watch: HangWatch): Promise<Ending> {
  try {
    const run = scopeOf(tool, request.script, keep).then((scope) => script.run(scope))
    const result = await withinLimit(watch.within(run), request.limitMs)
    if (result === OVERRAN) {
      return { kind: 'overran' }
    }
    if (result === HUNG) {
      return { kind: 'hung' }
    }
    if (result !== undefined) {
      printLines(tool.print, typeof result === 'string' ? result : inspect(result, { depth: 4 }))
    }
    return { kind: 'done' }
  } catch (error) {
    // The tool's own frames go to the server's log, never to the caller.
    console.error(error)
    return { kind: 'failed', error }
  }
}

/** Runs what helpers opened for the call to close when it ends, in reverse order, each for a bounded time. */
async function release(releases: (() => Promise<void>)[], print: (line: string) => void): Promise<void> {
  for (const release of releases.reverse()) {
    // What a helper opened for the call and cannot close, such as a browser that crashed, is gone already; the report says so.
    const released = await withinLimit(release(), REPORT_MS)
      .catch((error: unknown) => print(`Not released: ${error instanceof Error ? error.message.split('\n')[0] : String(error)}`))
    if (released === OVERRAN) {
      print(`Not released: no answer within ${REPORT_MS / 1000} s`)
    }
  }
}

/**
 * The page's state for the report, after a screenshot when the script
 * failed; null when the server has no page; `hung` when the browser does
 * not answer.
 */
async function reportPage(tool: Tool, failed: boolean, watch: HangWatch): Promise<PageState | null | 'hung'> {
  const page = await tool.browser.existingPage()
  if (!page) {
    return null
  }
  // Each step waits a bounded time, for a page that hangs while its browser still answers.
  if (failed) {
    const taken = await withinLimit(watch.within(failureShot(tool)), REPORT_MS)
    if (taken === OVERRAN || taken === HUNG) {
      return 'hung'
    }
  }
  try {
    const read = await withinLimit(watch.within(tool.browser.cdp().then((cdp) => readPageState(page, cdp))), REPORT_MS)
    return read === OVERRAN || read === HUNG ? 'hung' : read
  } catch (error) {
    // A page that is navigating or crashed has no state to read; the report says so instead.
    tool.print(`Page      could not be read: ${error instanceof Error ? error.message.split('\n')[0] : String(error)}`)
    return null
  }
}

/** The names the script runs with. */
async function scopeOf(tool: Tool, source: string, keep: Keep): Promise<ScriptScope> {
  const scope: ScriptScope = {
    page: undefined,
    context: undefined,
    cdp: undefined,
    expect,
    demi: createDemi(tool),
    keep,
    console: scriptConsole(tool.print),
  }
  if (USES_PAGE.test(source)) {
    // The browser reaches the web app through the slot's network.
    await tool.network()
    const page = await tool.browser.page()
    scope.page = addressed(page, webBase(tool.slot))
    scope.context = page.context()
    scope.cdp = await tool.browser.cdp()
  }
  return scope
}

/**
 * `page`, whose `goto` takes an address relative to the slot's web app.
 * Playwright resolves relative addresses against a context's `baseURL`,
 * which only a context Playwright creates can have, never the browser's own
 * context that a connection over the DevTools protocol attaches to, and a
 * context Playwright creates would lose the slot's profile; so `goto`
 * resolves them here, and every other member is the page's own.
 */
function addressed(page: Page, base: string): Page {
  return new Proxy(page, {
    get(target, property) {
      if (property === 'goto') {
        return (url: string, options?: Parameters<Page['goto']>[1]) => target.goto(new URL(url, base).href, options)
      }
      const value: unknown = Reflect.get(target, property, target)
      return typeof value === 'function' ? value.bind(target) : value
    },
  })
}

/** A console whose every line, log and error alike, goes to the caller in the order written. */
function scriptConsole(print: (line: string) => void): Console {
  const output = new Writable({
    write(chunk, _encoding, done) {
      printLines(print, String(chunk).replace(/\n$/, ''))
      done()
    },
  })
  return new Console({ stdout: output, stderr: output, colorMode: false })
}

function printLines(print: (line: string) => void, text: string): void {
  for (const line of text.split('\n')) {
    print(line)
  }
}

/** A picture of the page as the script left it, for the failure. */
async function failureShot(tool: Tool): Promise<void> {
  const path = shotPath(tool.slot, `failure-${Date.now()}`)
  try {
    writeFileSync(path, await tool.browser.capture())
    tool.wrote({ kind: 'failure', path, detail: pngSize(path) })
  } catch (error) {
    // A page that crashed or closed has no picture; the report says so.
    tool.print(`No screenshot: ${error instanceof Error ? error.message.split('\n')[0] : String(error)}`)
  }
}
