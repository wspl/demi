/**
 * A schedule as a conversation's Schedules menu lists it: what wakes the
 * conversation, when it next will, and how it last did. The user turns it off
 * and on and deletes it there; the agent does the rest. Its fields are the web
 * API's.
 */
export interface ConversationSchedule {
  id: string
  name: string
  trigger: Trigger
  /** The time zone its times are read in, such as Asia/Shanghai. */
  timeZone: string
  /** Whether the user or the agent turned it off; an archived conversation's schedules wake nothing either way. */
  paused: boolean
  /** When it next wakes the conversation, ISO 8601; none while it is off or its conversation archived, with no time left, or for a change watch or a URL. */
  nextAt: string | null
  /** The last wake, ISO 8601 `at`; none before it woke the conversation. */
  lastRun: LastRun | null
}

/** How a schedule last woke its conversation. */
export interface LastRun {
  at: string
  state: RunState
  note: RunNote | null
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
 * unit reads stay.
 */
export interface ScheduleTask {
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

/** A country by its ISO 3166 code, and one of its regions where its holidays differ. */
export interface HolidayPlace {
  country: string
  region: string | null
}

/** When a time trigger fires: once, on a repeating cycle with its tasks, every interval, or a cron expression of five fields. */
export type Timing =
  | { kind: 'once'; at: string }
  | { kind: 'repeat'; repeat: Repeat; tasks: ScheduleTask[] }
  | { kind: 'every'; minutes: number }
  | { kind: 'cron'; expression: string }

/** What starts a schedule's runs. */
export type Trigger =
  | { kind: 'time'; timing: Timing }
  | { kind: 'change'; command: string; everyMinutes: number }
  | { kind: 'url' }

/** Why a run was skipped or failed. */
export type RunNote =
  | { kind: 'offline'; device: string }
  | { kind: 'limit'; limit: number; calls: number }
  | { kind: 'command_timed_out' }
  | { kind: 'command_failed'; message: string }
  | { kind: 'turn_failed'; message: string; code: string | null }

export type RunState = 'running' | 'done' | 'notified' | 'skipped' | 'failed'
