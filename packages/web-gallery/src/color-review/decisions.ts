/**
 * Review decisions live outside the browser so the reviewer's answers can be read back
 * from the repo: the gallery dev server persists them to `.color-review/decisions.json`.
 */
export type ReviewChoice = 'accept' | 'keep' | 'instruct'

export interface ReviewDecision {
  choice: ReviewChoice
  note: string
  updatedAt: string
}

export type ReviewDecisions = Record<string, ReviewDecision>

const ENDPOINT = '/__color-review/decisions'
const CHOICES: readonly ReviewChoice[] = ['accept', 'keep', 'instruct']

function isDecision(value: unknown): value is ReviewDecision {
  if (typeof value !== 'object' || value === null) return false
  const record = value as Record<string, unknown>
  return (
    typeof record.choice === 'string'
    && CHOICES.includes(record.choice as ReviewChoice)
    && typeof record.note === 'string'
    && typeof record.updatedAt === 'string'
  )
}

function parseDecisions(raw: unknown): ReviewDecisions {
  if (typeof raw !== 'object' || raw === null) return {}
  const out: ReviewDecisions = {}
  for (const [id, value] of Object.entries(raw)) {
    if (isDecision(value)) out[id] = value
  }
  return out
}

export async function loadDecisions(): Promise<ReviewDecisions> {
  const response = await fetch(ENDPOINT)
  if (!response.ok) return {}
  return parseDecisions(await response.json())
}

export async function saveDecisions(decisions: ReviewDecisions): Promise<void> {
  const response = await fetch(ENDPOINT, {
    method: 'PUT',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(decisions, null, 2),
  })
  if (!response.ok) throw new Error(`save failed: ${response.status}`)
}
