import type { ModelSettings } from '../agent/model-selection'
import type { DeviceMark } from '../devices/mark'

/**
 * An automation as the page shows and edits it: what starts it, where it runs,
 * what it sends, and its runs, newest first. Its fields are the web API's, with
 * the target's place named as the host menu names it.
 */
export interface Automation {
  id: string
  name: string
  trigger: AutomationTrigger
  /** The time zone its times and its daily limit are read in, such as Asia/Shanghai. */
  timeZone: string
  /** The most runs a day, for a change watch or a URL; none without a limit. */
  maxRunsPerDay: number | null
  target: AutomationTarget
  /** What a single trigger's runs send; a repeat's tasks hold their own messages instead. */
  prompt: string
  /** The model settings a new conversation starts with; none for the user's own. */
  model: ModelSettings | null
  /** A paused one runs only with Run Now. */
  paused: boolean
  /** Why it paused itself; none when the user paused it or it is on. */
  pausedBecause: PauseReason | null
  /** A URL's address, `https://<server>/hooks/<id>`; none for another trigger. */
  address: string | null
  /** A URL trigger checks Stripe's signatures with the signing secret the user pasted. */
  stripeSigning: boolean
  /** When it next starts by itself, ISO 8601; none while paused, with no time left, or for a URL. */
  nextAt: string | null
  runs: AutomationRun[]
}

export type Weekday = 'mon' | 'tue' | 'wed' | 'thu' | 'fri' | 'sat' | 'sun'

/** Monday first, as the days a weekly task names are listed. */
export const WEEKDAYS: readonly Weekday[] = ['mon', 'tue', 'wed', 'thu', 'fri', 'sat', 'sun']

export type RepeatUnit = 'days' | 'weeks' | 'months' | 'years'

/**
 * A repeating cycle, as Calendar's repeat: every `every` days, weeks, months
 * or years. A cycle of several units starts on `start`, its first cycle;
 * every task runs in each cycle's first unit.
 */
export interface Repeat {
  every: number
  unit: RepeatUnit
  /** The first cycle's day, as 2026-01-01; none for a cycle of one unit. */
  start: string | null
}

/** Which of a month's weekdays: the first to the fourth, or the last. */
export type WeekOfMonth = 'first' | 'second' | 'third' | 'fourth' | 'last'

/**
 * A day of a month: a date, which a month without it skips; the last day;
 * a weekday's first to fourth or last; or the first or last working day.
 */
export type MonthDay =
  | { kind: 'date'; date: number }
  | { kind: 'last_day' }
  | { kind: 'weekday'; week: WeekOfMonth; weekday: Weekday }
  | { kind: 'working_day'; which: 'first' | 'last' }

/**
 * A task: a moment in the repeat's cycle and the message it sends. Its moment
 * is the time and, as the repeat's unit asks, its weekdays (weeks), its day
 * of the month (months), or its month and day (years); the parts another
 * unit reads stay, so changing the repeat back finds them as they were.
 */
export interface AutomationTask {
  id: string
  /** As 09:00. */
  time: string
  days: Weekday[]
  day: MonthDay
  /** 1 to 12. */
  month: number
  /**
   * The country, and its region, whose working week and public holidays a
   * first or last working day follows; none for Monday to Friday.
   */
  holidays: HolidayPlace | null
  /** Empty until written; a task without a message does not run. */
  prompt: string
}

/**
 * Whether the task has a message to send: one of only spaces or line breaks
 * has none, as the backend reads it, so it shows as missing and cannot run.
 */
export function hasMessage(task: Pick<AutomationTask, 'prompt'>): boolean {
  return task.prompt.trim() !== ''
}

/** When a time trigger fires: once, on a repeating cycle with its tasks, every interval, or a cron expression of five fields. */
export type TimeSchedule =
  | { kind: 'once'; at: string }
  | { kind: 'repeat'; repeat: Repeat; tasks: AutomationTask[] }
  | { kind: 'every'; minutes: number }
  | { kind: 'cron'; expression: string }

/** A country by its ISO 3166 code, and one of its regions where its holidays differ. */
export interface HolidayPlace {
  country: string
  region: string | null
}

/** A country whose public holidays Demi knows, with the regions whose holidays differ. */
export interface HolidayCountry {
  /** ISO 3166-1 alpha-2, which names it in the page's language. */
  code: string
  regions: readonly { code: string; name: string }[]
}

/** What starts an automation's runs. */
export type AutomationTrigger =
  | { kind: 'time'; schedule: TimeSchedule }
  | { kind: 'change'; command: string; everyMinutes: number }
  | { kind: 'url' }

/**
 * Where an automation's runs work. It is one of:
 *
 * - a new conversation each run, on a Host and in a project on it or none, or
 * - an existing conversation, which has its own place, and whose run's
 *   message, while it is busy, waits for its turn to end (queue) or reaches
 *   the running turn (steer), as the composer's Steer does.
 */
export type AutomationTarget =
  | { kind: 'new'; hostId: string; projectId: string | null }
  | { kind: 'existing'; conversationId: string; ifBusy: IfBusy }

export type IfBusy = 'queue' | 'steer'

export type PauseReason = 'conversation_archived' | 'conversation_deleted'

/** Why a run was skipped or failed. */
export type RunNote =
  | { kind: 'offline'; device: string }
  | { kind: 'limit'; limit: number; calls: number }
  | { kind: 'target_gone' }
  | { kind: 'no_model' }
  | { kind: 'command_timed_out' }
  | { kind: 'command_failed'; message: string }
  | { kind: 'turn_failed'; message: string; code: string | null }

/** One run: what the trigger started, and how it ended. */
export interface AutomationRun {
  id: string
  /** When it started, ISO 8601. */
  at: string
  /** The repeat's task that started it; none for a single trigger. */
  taskId: string | null
  state: RunState
  note: RunNote | null
  /** The start of the agent's last answer; none before it answered. */
  line: string | null
  /** The run's conversation; none for a run that started none, such as a skipped one. */
  conversationId: string | null
  /** A run that notified, whose conversation the user has not opened since. */
  unread: boolean
  /** Its new conversation is still filed under the automation, out of the sidebar. */
  filed: boolean
}

export type RunState = 'running' | 'done' | 'notified' | 'skipped' | 'failed'

/** A Host a target can name, as the host menu lists them. */
export interface AutomationHost {
  /** `cloud`, or the device's id. */
  id: string
  name: string
  kind: 'cloud' | 'device'
  state: 'online' | 'offline' | 'updating'
  /** A device's mark, with the number it shows (`shownMark`); null for the Cloud. */
  mark: DeviceMark | null
  /** Its projects. */
  projects: readonly { id: string; name: string }[]
}

/** A conversation an automation can wake, with the Host it runs on. */
export interface AutomationConversation {
  id: string
  title: string
  hostId: string
}

/** An automation as a conversation it wakes lists it: its chip's menu. */
export interface AutomationLink {
  id: string
  name: string
  /** Its trigger in words: Weekdays 09:00. */
  trigger: string
  paused: boolean
}

/** A change of an automation's parts, as the editor makes it; the Stripe secret is written, never read. */
export type AutomationChange = Partial<Pick<Automation, 'name' | 'trigger' | 'timeZone' | 'maxRunsPerDay' | 'target' | 'prompt' | 'model' | 'paused'>> & {
  /** Stripe's signing secret the user pasted; null to stop checking Stripe's signatures. */
  stripeSecret?: string | null
}
