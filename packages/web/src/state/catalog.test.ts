import { expect, test } from 'bun:test'
import { accountPlan } from './catalog'

// The plan the context card's usage limits line and Settings name
// (`usage-and-quota.md` § What a snapshot says): the snapshot's, else the
// one a sign-in recorded, which only Claude Code's detail is.

const snapshot = (plan: { id: string, label: string } | null) => ({
  plan,
  accountLabel: null,
  observedAt: '2026-10-12T04:00:00.000Z',
  source: 'probe' as const,
  windows: [],
})

test.each([
  { scenario: 'the snapshot names it', family: 'codex', detail: 'chatgpt', quota: snapshot({ id: 'pro', label: 'Pro' }), plan: 'Pro' },
  { scenario: 'Claude’s usage answer names none, its sign-in recorded max', family: 'claude-code', detail: 'max', quota: snapshot(null), plan: 'Max' },
  { scenario: 'a Claude account not probed yet, on a team plan', family: 'claude-code', detail: 'team', quota: null, plan: 'Team' },
  { scenario: 'how a Codex account signed in is not a plan', family: 'codex', detail: 'chatgpt', quota: null, plan: '' },
  { scenario: 'nothing says', family: 'claude-code', detail: null, quota: snapshot(null), plan: '' },
])('$scenario: "$plan"', ({ family, detail, quota, plan }) => {
  expect(accountPlan(family, { detail, quota })).toBe(plan)
})
