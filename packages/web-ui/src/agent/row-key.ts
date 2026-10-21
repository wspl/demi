import type { MessageListBlock } from './pending-steers'
import { sentMessage } from './message-delivery'
import { receivedAsBubble } from './agent-receipts'
import { isWorkStep, workRowId } from './work-groups'

/**
 * What names a row of the list across the forms it takes.
 *
 * - A message is one row through its whole delivery (`sentMessage`), named by
 *   the id it was sent with, which the message written keeps as its turn's id:
 *   on its way, waiting as a steer or in the queue, starting its turn, and
 *   written.
 * - A run is named after the run the backend stored, and a step that runs after
 *   itself. The row a lone step ran in becomes its run's row by taking its
 *   place (`succeedKeys`).
 *
 * The list keeps a row it names the same, with the height it measured, and
 * changes it in place, rather than building a new one that it lays out at a
 * guess for a frame. A `FunctionalBlock` row that replaces another of another
 * name takes the key the list draws it under (`succeedKeys`).
 */
export function rowKey(block: MessageListBlock): string {
  if (isWorkStep(block)) {
    return workRowId(block)
  }
  const message = sentMessage(block)
  return message ? `message:${message.id}` : block.id
}

/**
 * Whether the list draws `row` with `FunctionalBlock`, the 28px face of the
 * agent's work and the tail's state: a step or a run, a command's notice, an
 * agent's receipt, the Stopped row, the wait row (Requesting or Connecting)
 * and a share's note of steps not shared (`NoticeRow`). Such a row enters as
 * chrome (`chromeEntrance`) and rolls into another such row (`rowFace`).
 * Every other row, the agent's text, a message bubble (another agent's
 * message included, `receivedAsBubble`), an error, a compaction rule, appears
 * and changes in place.
 */
export function drawnAsFunctionalBlock(row: MessageListBlock): boolean {
  switch (row.type) {
    case 'agent_message':
      return !receivedAsBubble(row)
    case 'wakeup':
    case 'thinking':
    case 'tool_call':
    case 'pending_call':
    case 'work_group':
    case 'abort':
    case 'activity_slot':
    case 'steps_not_shared':
      return true
    default:
      return false
  }
}

/**
 * What a row shows, as far as a change of it is a new face; null for a row
 * not drawn with `FunctionalBlock`, which never rolls.
 *
 * A row whose face changes from one `FunctionalBlock` face to another under
 * the key it keeps rolls the new face over the old one (`RowRoll`), as the
 * Stopped row rolls into Requesting and Requesting into the step it waited
 * for. A run is one face from its first step, while it runs, on: a step that
 * ends becomes its run's row without a roll.
 */
export function rowFace(row: MessageListBlock): string | null {
  if (!drawnAsFunctionalBlock(row)) {
    return null
  }
  return isWorkStep(row) || row.type === 'work_group' ? 'work' : row.type
}

/**
 * A row of the list: the key it names itself by, and whether it rolls
 * (`rowFace`), which alone gives and takes a key.
 */
export interface ListRow {
  natural: string
  rolls: boolean
}

/** A row the list drew, with the key it drew it under. */
export interface DrawnRow extends ListRow {
  drawn: string
}

/**
 * The keys the list draws `next` under, after it drew `previous`.
 *
 * - A row the list drew before keeps the key it was drawn under, wherever it
 *   moved.
 * - A row that rolls and is gone gives its key to the first new row that rolls
 *   and stands where it stood, between the same rows that stayed in their
 *   order. The new state takes the old one's place, its height and its
 *   entrance, and rolls over it rather than one row going while another
 *   appears: Stopped and the Requesting row of the Continue, Requesting and the
 *   step it waited for, a call being written and its call.
 * - A row gone with no new row that rolls in its place goes, as the Requesting
 *   row of a Continue when the step it waited for joins the run above it, or
 *   Requesting when the answer's text or an error comes, which appears as a
 *   row of its own.
 * - A row that does not roll neither gives nor takes a key.
 * - A new row whose own key another row is drawn under takes a key of its own.
 */
export function succeedKeys(
  previous: readonly DrawnRow[],
  next: readonly ListRow[],
): string[] {
  if (previous.length === next.length && previous.every((row, index) => row.natural === next[index]!.natural)) {
    return previous.map((row) => row.drawn)
  }
  const before = new Map(previous.map((row, index) => [row.natural, { row, index }]))
  const staying = new Set(next.map((row) => row.natural))
  // The rows that stayed in their order stand still; a row that moved, such
  // as a steer the turn took, stands nowhere a row can be replaced.
  const still = inOrder(next.flatMap((row) => {
    const found = before.get(row.natural)
    return found ? [{ natural: row.natural, index: found.index }] : []
  }))
  const given = new Map<string, string[]>()
  let gap = ''
  for (const row of previous) {
    if (still.has(row.natural)) {
      gap = row.drawn
    } else if (!staying.has(row.natural) && row.rolls) {
      given.set(gap, [...(given.get(gap) ?? []), row.drawn])
    }
  }
  const drawn: string[] = []
  const used = new Set(next.flatMap((row) => before.get(row.natural)?.row.drawn ?? []))
  gap = ''
  for (const row of next) {
    const kept = before.get(row.natural)?.row.drawn
    if (kept !== undefined) {
      drawn.push(kept)
      if (still.has(row.natural)) {
        gap = kept
      }
      continue
    }
    // A key a new row took by its own name before is not given again.
    const offered = row.rolls ? given.get(gap)?.shift() : undefined
    const inherited = offered !== undefined && !used.has(offered) ? offered : undefined
    let key = inherited ?? row.natural
    for (let suffix = 2; inherited === undefined && used.has(key); suffix += 1) {
      key = `${row.natural}~${suffix}`
    }
    used.add(key)
    drawn.push(key)
  }
  return drawn
}

/**
 * The rows, of those that stayed, that kept their order: the longest run of
 * them in the order they were drawn before, so a row that moved leaves the
 * others standing.
 */
function inOrder(rows: readonly { natural: string; index: number }[]): Set<string> {
  // Patience sorting: `tails[n]` ends the best increasing run of length n + 1.
  const tails: number[] = []
  const links: number[] = []
  rows.forEach((row, at) => {
    let low = 0
    let high = tails.length
    while (low < high) {
      const middle = (low + high) >> 1
      if (rows[tails[middle]!]!.index < row.index) {
        low = middle + 1
      } else {
        high = middle
      }
    }
    links[at] = low > 0 ? tails[low - 1]! : -1
    tails[low] = at
  })
  const kept = new Set<string>()
  for (let at = tails.at(-1) ?? -1; at >= 0; at = links[at]!) {
    kept.add(rows[at]!.natural)
  }
  return kept
}
