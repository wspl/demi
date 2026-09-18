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
    items: [{ type: 'user_message', content: [{ type: 'text', text: message }] }],
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
