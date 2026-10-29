import type { Block } from '@demicodes/protocol'
import type { SidebarConversation } from '@demicodes/web-ui/sidebar/types'
import type { ConversationSchedule } from '@demicodes/web-ui/schedules/types'
import { demoModel, inRequests } from './blocks'
import { GALLERY_MARKS } from './device-marks'
import { DAY, HOUR, MINUTE, ago, ahead } from './time'

/** The next `weekday` (0 Sunday to 6 Saturday) at `hour`:00 local time, whatever day the gallery opens on. */
function nextWeekday(weekday: number, hour: number): string {
  const moment = new Date()
  moment.setHours(hour, 0, 0, 0)
  while (moment.getTime() <= Date.now() || moment.getDay() !== weekday) {
    moment.setDate(moment.getDate() + 1)
  }
  return moment.toISOString()
}

/**
 * The schedules of a conversation, as its Schedules window lists them: a
 * weekly report, a change watch, a once schedule, a paused one, and one whose
 * last wake failed because its Host was offline.
 */
export function gallerySchedules(): ConversationSchedule[] {
  const weekly = nextWeekday(1, 10)
  const reminder = ahead(21 * HOUR)
  return [
    {
      id: 'sc-report',
      name: 'Weekly report',
      trigger: {
        kind: 'time',
        timing: {
          kind: 'repeat',
          repeat: { every: 1, unit: 'weeks', start: null },
          tasks: [{ id: 'sc-report-1', time: '10:00', days: ['mon'], day: { kind: 'date', date: 1 }, month: 1, holidays: null, prompt: 'Write the weekly report.' }],
        },
      },
      timeZone: 'Asia/Shanghai',
      paused: false,
      nextAt: weekly,
      lastRun: { at: ago(5 * DAY), state: 'done', note: null },
    },
    {
      id: 'sc-ci',
      name: 'CI on main',
      trigger: { kind: 'change', command: 'gh run list --branch main --limit 1', everyMinutes: 5 },
      timeZone: 'Asia/Shanghai',
      paused: false,
      nextAt: null,
      lastRun: { at: ago(3 * MINUTE), state: 'notified', note: null },
    },
    {
      id: 'sc-remind',
      name: 'Remind me',
      trigger: { kind: 'time', timing: { kind: 'once', at: reminder } },
      timeZone: 'Asia/Shanghai',
      paused: false,
      nextAt: reminder,
      lastRun: null,
    },
    {
      id: 'sc-cleanup',
      name: 'Nightly cleanup',
      trigger: { kind: 'time', timing: { kind: 'cron', expression: '0 2 * * *' } },
      timeZone: 'Asia/Shanghai',
      paused: true,
      nextAt: null,
      lastRun: { at: ago(2 * DAY), state: 'done', note: null },
    },
    {
      id: 'sc-sync',
      name: 'Sync the photos',
      trigger: { kind: 'time', timing: { kind: 'every', minutes: 30 } },
      timeZone: 'Asia/Shanghai',
      paused: false,
      nextAt: ahead(12 * MINUTE),
      lastRun: { at: ago(18 * MINUTE), state: 'failed', note: { kind: 'offline', device: 'Mac mini' } },
    },
  ]
}

/**
 * `schedule` turned off, or with `paused` false on, as the backend answers
 * the switch: a schedule that is off has no next time, and one turned on
 * counts from now, which a guess at its next time stands for here.
 */
export function switched(schedule: ConversationSchedule, paused: boolean): ConversationSchedule {
  if (paused || schedule.trigger.kind !== 'time') {
    return { ...schedule, paused, nextAt: null }
  }
  const timing = schedule.trigger.timing
  const nextAt = timing.kind === 'once'
    ? timing.at
    : timing.kind === 'every' ? ahead(timing.minutes * MINUTE) : ahead(5 * HOUR)
  return { ...schedule, paused, nextAt }
}

/** The conversations the schedules wake, as the sidebar lists them, an alarm clock marking each. */
export function wokenConversations(): SidebarConversation[] {
  return [
    {
      id: 'c-price-watch',
      title: 'Price watch',
      updatedAt: ago(3 * MINUTE),
      status: 'done',
      projectId: null,
      pinned: false,
      unread: false,
      scheduled: true,
      device: { kind: 'paired', name: 'MacBook Pro', state: 'online', mark: GALLERY_MARKS.mac },
    },
    {
      id: 'c-cloud-orders',
      title: 'Orders dashboard',
      updatedAt: ago(DAY),
      status: 'idle',
      projectId: null,
      pinned: false,
      unread: false,
      scheduled: true,
    },
  ]
}

/** The schedules of each woken conversation: a change watch for one, the whole list for the other. */
export function schedulesOfConversations(): Record<string, ConversationSchedule[]> {
  const [report, ci, remind, cleanup, sync] = gallerySchedules() as [ConversationSchedule, ConversationSchedule, ConversationSchedule, ConversationSchedule, ConversationSchedule]
  return {
    'c-price-watch': [ci],
    'c-cloud-orders': [report, remind, cleanup, sync],
  }
}

/** A clock time: 09:00. */
function clockOf(iso: string): string {
  const moment = new Date(iso)
  return `${String(moment.getHours()).padStart(2, '0')}:${String(moment.getMinutes()).padStart(2, '0')}`
}

/**
 * A conversation a schedule wakes: the user's own start, then the schedule's
 * message as the backend writes it, which the transcript marks with the
 * schedule, and the agent's answer.
 */
export function wokenConversationBlocks(id: string, schedule: ConversationSchedule): Block[] {
  const at = ago(40 * MINUTE)
  const time = clockOf(at)
  const head = schedule.trigger.kind === 'change'
    ? `Schedule “${schedule.name}”, its command’s output changed at ${time}`
    : `Schedule “${schedule.name}”, at ${time}`
  const extra = schedule.trigger.kind === 'change'
    ? ['', 'The output before (exit status 0):', '```text', '1349', '```', '', 'The output now (exit status 0):', '```text', '1329', '```']
    : []
  return inRequests([
    {
      type: 'user', id: `${id}-start`, turnId: `${id}-start-turn`, createdAt: ago(3 * DAY), model: demoModel,
      content: [{ type: 'text', text: 'Keep an eye on this for me; set up a schedule to wake yourself.' }], preamble: null,
    },
    { type: 'text', id: `${id}-start-answer`, createdAt: ago(3 * DAY - 30_000), endedAt: ago(3 * DAY - 30_000), model: demoModel, text: 'Sure. Each time it wakes me I’ll compare with what we have here.' },
    {
      type: 'user', id: `${id}-message`, turnId: `${id}-turn`, createdAt: at, model: demoModel,
      content: [{ type: 'text', text: [head, '', 'Compare with the last time and tell me what changed.', ...extra].join('\n') }], preamble: null,
      schedule: { scheduleId: schedule.id, runId: `${schedule.id}-run`, name: schedule.name, at },
    },
    { type: 'text', id: `${id}-answer`, createdAt: ago(40 * MINUTE - 40_000), endedAt: ago(40 * MINUTE - 40_000), model: demoModel, text: 'Nothing new since the last check; the count went from 1349 to 1329.' },
  ])
}
