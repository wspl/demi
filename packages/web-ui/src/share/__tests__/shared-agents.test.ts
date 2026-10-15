import { describe, expect, test } from 'bun:test'
import { sharedMessageAgents } from '../shared-agents'

// A shared transcript’s links (`sharing.md` § What a share holds). Pure, under a millisecond.

describe('a shared transcript’s agent links', () => {
  const agents = [
    { number: 2, description: 'Read the logs', profile: null },
    { number: 5, description: '  ', profile: 'researcher' },
  ]

  test('find the conversation’s own agent and the subagents the snapshot holds, by their description or number', () => {
    const links = sharedMessageAgents(agents, () => {})
    expect(links.find(0)?.number).toBe(0)
    expect(links.find(2)?.description).toBe('Read the logs')
    expect(links.find(5)?.description).toBe('Agent 5')
    expect(links.find(3)).toBeNull()
  })

  test('open only an agent the snapshot holds, or the conversation’s own', () => {
    const opened: number[] = []
    const links = sharedMessageAgents(agents, (number) => opened.push(number))
    links.open(5)
    links.open(3)
    links.open(0)
    expect(opened).toEqual([5, 0])
  })
})
