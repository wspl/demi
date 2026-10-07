// `bun browse wait …`: waits for what happens, never for time
// (browse.md § Waiting): an element to appear or go, text, an
// address, a JavaScript condition, or the end of the open conversation's
// turn. Every wait has a timeout, and its failure says what it waited for.
import { errors } from 'playwright'
import { CommandFailure, parse, timeoutMs, timeoutOption, type Context } from '../command'
import { failure } from '../find'
import { locate, parseTarget } from '../locator'
import { matcher } from '../pattern'
import { conversationId, summary, waitTurnEnd } from '../turn'

const USAGE = 'wait <locator> | gone <locator> | text <text> | url <pattern> | js <expression> | turn [--timeout <s>]'

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals, values } = parse(argv, timeoutOption, USAGE)
  const [first, second] = positionals
  if (first === undefined || positionals.length > 2) {
    throw new CommandFailure(`Usage: bun browse ${USAGE}`)
  }
  const page = await context.browser.page()
  const started = Date.now()
  if (first === 'turn' && second === undefined) {
    const id = conversationId(page)
    if (!id) {
      throw new CommandFailure(`The page shows no conversation: ${page.url()}`)
    }
    if (await summary(context, page, id) === null) {
      throw new CommandFailure(`Conversation ${id} has no message yet, so it has no turn`)
    }
    const end = await waitTurnEnd(context, page, id, { timeoutMs: timeoutMs(values.timeout, 600) })
    context.print(`The turn ended after ${seconds(started)}: ${end.status}`)
    return
  }
  const timeout = timeoutMs(values.timeout, 30)
  const [what, waiting] = condition(first, second)
  try {
    await waiting(timeout)
  } catch (error) {
    if (error instanceof errors.TimeoutError) {
      throw await failure(context, [`Waited ${timeout / 1000} s for ${what}; the page is at ${page.url()}.`])
    }
    throw error
  }
  context.print(`${what}: after ${seconds(started)}`)

  /** What the wait is for, in words, and the wait itself. */
  function condition(kind: string, argument: string | undefined): [string, (timeout: number) => Promise<unknown>] {
    const needs = (value: string | undefined): string => {
      if (value === undefined) {
        throw new CommandFailure(`Usage: bun browse ${USAGE}`)
      }
      return value
    }
    switch (kind) {
      case 'gone': {
        const text = needs(argument)
        return [`${text} to go`, (timeout) => locator(text).waitFor({ state: 'hidden', timeout })]
      }
      case 'text': {
        const text = needs(argument)
        return [`the text ${JSON.stringify(text)}`, (timeout) => page.getByText(text).first().waitFor({ state: 'visible', timeout })]
      }
      case 'url': {
        const pattern = needs(argument)
        const matches = matcher(pattern)
        return [`an address matching ${pattern}`, (timeout) => page.waitForURL((url) => matches(url.href), { timeout })]
      }
      case 'js': {
        const expression = needs(argument)
        return [`${expression} to hold`, (timeout) => page.waitForFunction(expression, undefined, { timeout, polling: 'raf' })]
      }
      default: {
        if (argument !== undefined) {
          throw new CommandFailure(`Usage: bun browse ${USAGE}`)
        }
        return [`${kind} to appear`, (timeout) => locator(kind).waitFor({ state: 'visible', timeout })]
      }
    }
  }

  function locator(text: string) {
    const target = parseTarget(text)
    if (target.kind === 'point') {
      throw new CommandFailure(`${text} is a point; wait needs a locator`)
    }
    return locate(page, target.segments)
  }
}

function seconds(since: number): string {
  return `${((Date.now() - since) / 1000).toFixed(1)} s`
}
