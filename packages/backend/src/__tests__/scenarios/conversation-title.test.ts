import { afterAll, beforeAll, expect, test } from 'bun:test'
import { TITLE_INSTRUCTION } from '../../conversation/title'
import { World } from './world'
import { model } from './driver'

// Conversation titles (product.md § Conversation titles): the first send gives
// the message-derived title at once and starts one model request beside the
// turn; its line replaces the title unless the user renamed meanwhile.

let world: World

beforeAll(async () => {
  world = await World.create({})
})

afterAll(async () => {
  await world.close()
})

async function titleOf(id: string): Promise<string> {
  const state = await world.api<{ conversations: Array<{ id: string; title: string }> }>('/api/state')
  return state.conversations.find(conversation => conversation.id === id)!.title
}

async function waitForTitle(id: string, title: string): Promise<void> {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    if (await titleOf(id) === title)
      return
    await Bun.sleep(20)
  }
  expect(await titleOf(id)).toBe(title)
}

test('the first send titles from the message, then from the model; later sends ask for nothing', async () => {
  const driver = await world.conversation('cloud')
  const message = 'why does   pnpm build\nfail with TS2307 after I moved auth into its own package'
  world.model.scriptTitle(driver.id, '\n"pnpm build TS2307 after auth package move"\nIgnored second line')
  await driver.turn({ text: message, model: [model.say('looking')] })
  await waitForTitle(driver.id, 'pnpm build TS2307 after auth package move')

  const requests = world.model.titleRequests.filter(request => request.sessionId === driver.id)
  expect(requests).toHaveLength(1)
  expect(requests[0]).toMatchObject({
    systemPrompt: TITLE_INSTRUCTION,
    tools: [],
    serviceTierId: null,
    // The message on one line, numbered: the one the user has sent so far.
    items: [{ type: 'user_message', content: [{ type: 'text', text: '1. why does pnpm build fail with TS2307 after I moved auth into its own package' }] }],
  })
  // The turn's own request carried the agent's prompt, not the title's.
  expect(world.model.requests.every(request => request.systemPrompt !== TITLE_INSTRUCTION)).toBe(true)

  await driver.turn({ text: 'and now something else entirely', model: [model.say('ok')] })
  expect(world.model.titleRequests.filter(request => request.sessionId === driver.id)).toHaveLength(1)
  expect(await titleOf(driver.id)).toBe('pnpm build TS2307 after auth package move')
})

test('a title request that yields nothing leaves the message-derived title', async () => {
  const driver = await world.conversation('cloud')
  await driver.turn({ text: 'hello   there', model: [model.say('hi')] })
  expect(world.model.titleRequests.some(request => request.sessionId === driver.id)).toBe(true)
  expect(await titleOf(driver.id)).toBe('hello there')
})

test('the browser repeating the placeholder at record creation settles nothing', async () => {
  const driver = await world.conversation('cloud')
  await world.api(`/api/conversations/${driver.id}`, { title: 'New conversation' }, 'PATCH')
  world.model.scriptTitle(driver.id, 'Deploy checklist')
  await driver.turn({ text: 'walk me through the deploy checklist', model: [model.say('ok')] })
  await waitForTitle(driver.id, 'Deploy checklist')
})

test('a rename that lands while the request is in flight wins', async () => {
  const driver = await world.conversation('cloud')
  const answer = Promise.withResolvers<string>()
  world.model.scriptTitle(driver.id, () => answer.promise)
  await driver.turn({ text: 'set up the release pipeline', model: [model.say('ok')] })
  expect(await titleOf(driver.id)).toBe('set up the release pipeline')

  await world.api(`/api/conversations/${driver.id}`, { title: 'Release work' }, 'PATCH')
  answer.resolve('Release pipeline setup')
  await Bun.sleep(100)
  expect(await titleOf(driver.id)).toBe('Release work')
})

test('the user asks for a new title: every message, each cut short, and the button only while a message is newer than the title', async () => {
  const driver = await world.conversation('cloud')
  const summary = async () => {
    const state = await world.api<{ conversations: Array<{ id: string, title: string, titleCurrent: boolean, titleGenerating: boolean }> }>('/api/state')
    return state.conversations.find(conversation => conversation.id === driver.id)!
  }
  world.model.scriptTitle(driver.id, 'Release pipeline')
  await driver.turn({ text: 'set up the release pipeline', model: [model.say('ok')] })
  await waitForTitle(driver.id, 'Release pipeline')
  // The first title saw the only message: nothing to ask for.
  expect(await summary()).toMatchObject({ titleCurrent: true, titleGenerating: false })

  await driver.turn({ text: `now fix the login test ${'x'.repeat(600)}`, model: [model.say('ok')] })
  expect(await summary()).toMatchObject({ title: 'Release pipeline', titleCurrent: false })

  const answer = Promise.withResolvers<string>()
  world.model.scriptTitle(driver.id, () => answer.promise)
  await world.api(`/api/conversations/${driver.id}/title`, { provider: world.selection })
  expect(await summary()).toMatchObject({ titleCurrent: false, titleGenerating: true })
  // Asking again while it runs joins the request.
  await world.api(`/api/conversations/${driver.id}/title`, { provider: world.selection })
  answer.resolve('Login test fix')
  await waitForTitle(driver.id, 'Login test fix')
  expect(await summary()).toMatchObject({ titleCurrent: true, titleGenerating: false })

  const requests = world.model.titleRequests.filter(request => request.sessionId === driver.id)
  expect(requests).toHaveLength(2)
  const input = requests[1]!.items[0]
  expect(input).toMatchObject({ type: 'user_message' })
  const text = input?.type === 'user_message' && input.content[0]?.type === 'text' ? input.content[0].text : ''
  const lines = text.split('\n')
  expect(lines[0]).toBe('1. set up the release pipeline')
  expect(lines[1]).toStartWith('2. now fix the login test xxx')
  expect(lines[1]!.length).toBe('2. '.length + 400)
  expect(requests[1]!.tools).toEqual([])

  // A title that fails leaves the button up: pressing it again is the retry.
  await driver.turn({ text: 'and the docs', model: [model.say('ok')] })
  world.model.scriptTitle(driver.id, '')
  await world.api(`/api/conversations/${driver.id}/title`, { provider: world.selection })
  await Bun.sleep(100)
  expect(await summary()).toMatchObject({ title: 'Login test fix', titleCurrent: false, titleGenerating: false })
})

test('a conversation with no message has no title to ask for', async () => {
  const driver = await world.conversation('cloud')
  const response = await world.backend.session.fetch(`/api/conversations/${driver.id}/title`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ provider: world.selection }),
  })
  expect(response.status).toBe(409)
  expect(await response.json()).toMatchObject({ code: 'no_messages' })
})
