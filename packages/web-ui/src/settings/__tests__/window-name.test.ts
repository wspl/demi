import { afterEach, expect, test } from 'bun:test'
import type { QuotaKind, QuotaWindowName } from '@demicodes/protocol'
import { createPageI18n } from '../../i18n/text'
import { windowName, windowNameInSentence } from '../window-name'

// A usage window reads by its kind, its period and its scope in the page's
// language, the same for every vendor (`packages/web-ui/src/ui/dates.ts`), and
// a sentence names it in lowercase but for a model, with room
// beside Chinese text. Microseconds.

const HOUR = 3600
const DAY = 24 * HOUR
const WEEK = 7 * DAY

function named(kind: QuotaKind, periodSeconds: number | null, model: string | null = null): QuotaWindowName {
  return { kind, periodSeconds, scope: model === null ? null : { kind: 'model', label: model } }
}

afterEach(() => {
  createPageI18n('en')
})

const windows: QuotaWindowName[] = [
  named('limit', 5 * HOUR),
  named('limit', DAY),
  named('limit', WEEK),
  named('limit', 3 * DAY),
  named('limit', 90 * 60),
  named('limit', WEEK, 'Opus'),
  named('limit', null),
  named('limit', null, 'Opus'),
  named('credits', WEEK),
  named('credits', 31 * DAY),
  named('credits', 28 * DAY),
  named('credits', null),
  named('requests', null),
  named('tokens', null),
  named('cap', null),
]

test('a window reads by its kind, its period and its model, in English and in Chinese', () => {
  expect(windows.map(windowName)).toEqual([
    '5-hour limit',
    'Daily limit',
    'Weekly limit',
    '3-day limit',
    '90-minute limit',
    'Weekly Opus limit',
    'Usage limit',
    'Opus limit',
    'Weekly credits',
    'Monthly credits',
    'Monthly credits',
    'Credits',
    'Request rate',
    'Token rate',
    'On-demand cap',
  ])
  createPageI18n('zh-Hans')
  expect(windows.map(windowName)).toEqual([
    '5 小时限额',
    '每日限额',
    '每周限额',
    '3 天限额',
    '90 分钟限额',
    '每周 Opus 限额',
    '用量限额',
    'Opus 限额',
    '每周额度',
    '每月额度',
    '每月额度',
    '额度',
    '请求速率',
    'Token 速率',
    '按需用量上限',
  ])
})

test('inside a sentence a name is lowercase but for its model, and stands apart from Chinese text', () => {
  expect(windowNameInSentence(named('limit', WEEK, 'Opus'))).toBe('weekly Opus limit')
  expect(windowNameInSentence(named('limit', 5 * HOUR))).toBe('5-hour limit')
  expect(windowNameInSentence(named('limit', null, 'Opus'))).toBe('Opus limit')
  createPageI18n('zh-Hans')
  expect(windowNameInSentence(named('limit', 5 * HOUR))).toBe(' 5 小时限额')
  expect(windowNameInSentence(named('limit', WEEK))).toBe('每周限额')
})
