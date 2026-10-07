// One call of `bun browse` (browse.md § A call): runs the script in its
// scope, prints what it logged and returned, and ends with the report:
// where and why it failed, if it did, with a screenshot, then the footer.
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

/** What the server lends a call. */
export interface Server {
  slot: Slot
  browser: Browser
  network(): Promise<Network>
  keep: Keep
}

export interface Outcome {
  exit: number
  /** Whether a helper asked the server to end once the call answered, as `demi.down` does. */
  endServer: boolean
  /**
   * Whether the script ran into the call's time limit: it still runs, and
   * only the end of the server's process stops it.
   */
  overran: boolean
}

/**
 * Names that make a script need the page: a script that uses neither, such
 * as `demi.down()`, starts no browser for nothing. A helper that needs the
 * page starts the browser itself.
 */
const USES_PAGE = /\b(page|context|cdp)\b/

/** What `withinLimit` answers when the limit came first. */
const OVERRAN = Symbol('overran')

export async function runCall(server: Server, request: Request, print: (line: string) => void): Promise<Outcome> {
  const { slot, browser } = server
  const shots: Shot[] = []
  let endServer = false
  const tool: Tool = {
    slot,
    browser,
    network: server.network,
    print,
    env: request.env,
    wrote: (shot) => shots.push(shot),
    endServer: () => {
      endServer = true
    },
  }
  // The page's problems since the call began are those logged after this entry.
  const since = browser.logs.newest
  const script = new Script(request.script, slotPaths(slot).calls)
  let ending: { kind: 'done' } | { kind: 'failed', error: unknown } | { kind: 'overran' } = { kind: 'done' }
  try {
    const result = await withinLimit(script.run(await scopeOf(tool, request.script, server.keep)), request.limitMs)
    if (result === OVERRAN) {
      ending = { kind: 'overran' }
    } else if (result !== undefined) {
      printLines(print, typeof result === 'string' ? result : inspect(result, { depth: 4 }))
    }
  } catch (error) {
    ending = { kind: 'failed', error }
    // The tool's own frames go to the server's log, never to the caller.
    console.error(error)
  } finally {
    script.remove()
  }

  if (ending.kind === 'overran') {
    print(`Failed: the script ran for ${request.limitMs / 1000} s, its time limit (--limit); the tool's server restarts to stop it and keeps the browser`)
  } else if (ending.kind === 'failed') {
    const line = script.lineOf(ending.error)
    const where = { line, text: line === null ? '' : script.text(line), file: request.file }
    for (const text of describeFailure(ending.error, where)) {
      print(text)
    }
  }
  const page = await browser.existingPage()
  if (page && ending.kind !== 'done') {
    await failureShot(tool)
  }
  let state: PageState | null = null
  if (page) {
    try {
      state = await readPageState(page, await browser.cdp())
    } catch (error) {
      // A page that is navigating or crashed has no state to read; the report says so instead.
      print(`Page      could not be read: ${error instanceof Error ? error.message.split('\n')[0] : String(error)}`)
    }
  }
  for (const line of composeFooter({ shots, page: state, problems: browser.logs.problems(since) })) {
    print(line)
  }
  return { exit: ending.kind === 'done' ? 0 : 1, endServer, overran: ending.kind === 'overran' }
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

/** Runs until `work` settles or `ms` pass, whichever comes first. */
async function withinLimit<T>(work: Promise<T>, ms: number): Promise<T | typeof OVERRAN> {
  let timer: ReturnType<typeof setTimeout> | undefined
  const limit = new Promise<typeof OVERRAN>((resolve) => {
    timer = setTimeout(() => resolve(OVERRAN), ms)
  })
  // Once the limit came first, the script's failure has no one to report
  // to: the server ends to stop it.
  work.catch(() => undefined)
  try {
    return await Promise.race([work, limit])
  } finally {
    clearTimeout(timer)
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
