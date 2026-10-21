import { expect, test } from 'bun:test'
import { useMoveQuestion, type MoveQuestion } from '../move-question'

// The dialog's state over a stubbed move: a few milliseconds.

const asked: MoveQuestion = { host: 'MacBook Pro', directory: '~', from: 'Cloud', fromCloud: true }

test('Cancel carries out nothing, and Move carries out the move once however often it is pressed', async () => {
  const question = useMoveQuestion()
  let carried = 0
  const carryOut = async () => {
    carried += 1
    return true
  }

  const cancelled = question.ask(asked, carryOut)
  question.cancel()
  expect(await cancelled).toBe(false)
  expect(carried).toBe(0)
  expect(question.open.value).toBe(false)

  const answered = question.ask(asked, carryOut)
  const again = question.answer()
  void question.answer()
  expect(await answered).toBe(true)
  await again
  expect(carried).toBe(1)
  expect(question.open.value).toBe(false)
})
