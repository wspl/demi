import { expect, test } from 'bun:test'
import { stopSentence, useMoveQuestion, workToStop, type MoveQuestion } from '../move-question'
import type { SubagentRecord } from '../../agent/subagents'
import type { TerminalRecord } from '../../agent/terminals'

// Pure functions and the dialog's state over stubbed answers: a few milliseconds.

const asked: MoveQuestion = { host: 'MacBook Pro', directory: '~', from: 'Cloud', fromCloud: true }

const command = (phase: TerminalRecord['phase']): TerminalRecord => ({
  id: phase, title: 'Build', phase, startedAt: '2026-09-09T00:00:00.000Z', output: '',
})
const subagent = (phase: SubagentRecord['phase']): Pick<SubagentRecord, 'phase'> => ({ phase })

test.each([
  { scenario: 'the agent and commands', work: { agent: true, subagents: 0, commands: 2 }, sentence: 'The agent and 2 commands will stop.' },
  { scenario: 'one command', work: { agent: true, subagents: 0, commands: 1 }, sentence: 'The agent and 1 command will stop.' },
  { scenario: 'the agent alone', work: { agent: true, subagents: 0, commands: 0 }, sentence: 'The agent will stop.' },
  { scenario: 'all three', work: { agent: true, subagents: 1, commands: 2 }, sentence: 'The agent, 1 subagent and 2 commands will stop.' },
  { scenario: 'a subagent after the turn ended', work: { agent: false, subagents: 2, commands: 0 }, sentence: '2 subagents will stop.' },
])('the question names what stops: $scenario', ({ work, sentence }) => {
  expect(stopSentence(work)).toBe(sentence)
})

test('what stops is counted from the phases the page shows; once everything ended nothing stops', () => {
  const at = (conversation: Omit<Parameters<typeof workToStop>[0], 'onlyWaitsForHost'>) =>
    workToStop({ ...conversation, onlyWaitsForHost: false })
  expect(at({ phase: 'idle', subagents: [subagent('completed')], terminals: [command('exited')] })).toBeNull()
  // A command that runs on after the turn ended still holds the conversation.
  expect(at({ phase: 'idle', subagents: [], terminals: [command('running')] }))
    .toEqual({ agent: false, subagents: 0, commands: 1 })
  expect(at({ phase: 'running', subagents: [subagent('running'), subagent('aborted')], terminals: [command('running'), command('exited')] }))
    .toEqual({ agent: true, subagents: 1, commands: 1 })
  expect(at({ phase: 'idle', subagents: [subagent('running')], terminals: [] }))
    .toEqual({ agent: false, subagents: 1, commands: 0 })
})

test('while the backend says the agent only waits for its offline device, a move stops nothing', () => {
  const waiting = { ...command('running'), waitingFor: 'MacBook Pro' }
  const conversation = { phase: 'running' as const, subagents: [], terminals: [waiting] }
  expect(workToStop({ ...conversation, onlyWaitsForHost: true })).toBeNull()
  // A command shown waiting is not enough: the backend decides.
  expect(workToStop({ ...conversation, onlyWaitsForHost: false }))
    .toEqual({ agent: true, subagents: 0, commands: 1 })
})

test('Cancel carries out nothing, and an answer carries out the move once, told or not', async () => {
  const question = useMoveQuestion()
  const carried: boolean[] = []
  const carryOut = async (tell: boolean) => {
    carried.push(tell)
    return true
  }

  const cancelled = question.ask(asked, carryOut)
  question.cancel()
  expect(await cancelled).toBe(false)
  expect(carried).toEqual([])
  expect(question.open.value).toBe(false)

  const answered = question.ask(asked, carryOut)
  const again = question.answer(true)
  void question.answer(false)
  expect(await answered).toBe(true)
  await again
  expect(carried).toEqual([true])
})
