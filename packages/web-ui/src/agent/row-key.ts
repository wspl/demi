import type { MessageListBlock } from './pending-steers'
import { isWorkStep, workRowId } from './work-groups'

/**
 * What names a row of the list across the forms it takes
 * (`product.md` § Following the agent's work): the message being sent and
 * the message the server confirmed are one row, named by the message's id,
 * which the confirmed message keeps as its turn's id; a steer waiting for
 * the turn and the steer written, and a queued message and the message it
 * becomes, likewise. A step shown alone is named as the run it starts, so
 * it is the same row as that run's group. The list keeps a row it names the
 * same, with the height it measured, and changes it in place, rather than
 * building a new one that it lays out at a guess for a frame. What replaces
 * a row of another name takes the key the list draws it under
 * (`succeedKeys`).
 */
export function rowKey(block: MessageListBlock): string {
  if (isWorkStep(block)) {
    return workRowId(block)
  }
  switch (block.type) {
    case 'user':
      return `message:${block.turnId}`
    case 'pending_submission':
      return `message:${block.submission.id}`
    case 'queued_message':
      return `message:${block.queueId}`
    case 'steer':
      return `message:${block.id}`
    case 'pending_steer':
      return `message:${block.pendingSteerId}`
    default:
      return block.id
  }
}

/**
 * What a row shows, as far as a change of it is a new face: a row whose face
 * changes under the key it keeps rolls the new face over the old one
 * (`RowRoll`), as the Stopped row rolls into Requesting and Requesting into
 * the step it waited for. A message is one face from its send to the
 * message written, and a run one face from its first step on, whose own
 * row rolls its steps.
 */
export function rowFace(row: MessageListBlock): string {
  if (isWorkStep(row) || row.type === 'work_group') {
    return 'work'
  }
  switch (row.type) {
    case 'user':
    case 'steer':
    case 'pending_steer':
    case 'queued_message':
    case 'pending_submission':
      return 'message'
    case 'compaction_marker':
    case 'compaction_boundary':
      return 'compaction'
    default:
      return row.type
  }
}

/** A row of the list: the key it names itself by, and whether it neither gives nor takes a key. */
export interface ListRow {
  natural: string
  fixed: boolean
}

/** A row the list drew, with the key it drew it under. */
export interface DrawnRow extends ListRow {
  drawn: string
}

/**
 * The keys the list draws `next` under, after it drew `previous`
 * (`product.md` § Following the agent's work: what replaces a row takes its
 * place). A row the list drew before keeps the key it was drawn under,
 * wherever it moved. A row that is gone gives its key to the first new row
 * that stands where it stood, between the same rows that stayed in their
 * order, so the new state takes the old one's place, its height and its
 * entrance, and rolls over it rather than one row going while another
 * appears: Stopped and the Requesting row of the Continue, Requesting and
 * the step it waited for, a call being written and its call. A row gone
 * with no new row in its place goes, as the Requesting row of a Continue
 * when the step it waited for joins the run above it. `fixed` rows,
 * such as the edges of a window of history, neither give nor take a key. A
 * new row whose own key another row is drawn under takes a key of its own.
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
    } else if (!staying.has(row.natural) && !row.fixed) {
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
    const offered = row.fixed ? undefined : given.get(gap)?.shift()
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
