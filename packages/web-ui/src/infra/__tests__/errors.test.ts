import { afterEach, expect, test } from 'bun:test'
import { computed, reactive } from 'vue'
import { createPageI18n } from '../../i18n/text'
import { coded, errorMessage, formError, reportError, reportPageErrorsWith, shownError, type PageErrorReport } from '../errors'
import { dismissToast, toasts } from '../toast'

// An error the page shows names the request it came from, and a failure the
// page found itself is reported to the backend, whose ID the toast then names
// (`backend.md` § Request IDs and the server log). A reporter stand-in in
// place of the backend; no I/O, a few milliseconds.

let stopReports: (() => void) | null = null

afterEach(() => {
  stopReports?.()
  stopReports = null
  for (const toast of [...toasts]) dismissToast(toast.id)
})

/** A request the backend refused, as the app's API errors carry it. */
class Refused extends Error {
  constructor(message: string, readonly requestId: string | null) {
    super(message)
  }
}

/** Records each report and answers with `answer`. */
function reporter(answer: string | null) {
  const reports: PageErrorReport[] = []
  stopReports = reportPageErrorsWith(async (report) => {
    reports.push(report)
    return answer
  })
  return reports
}

/** Lets the report's answer land. */
async function settle(): Promise<void> {
  for (let tick = 0; tick < 5; tick++)
    await Promise.resolve()
}

test('a failed request is shown with its ID and is not reported again', async () => {
  const reports = reporter('req_unused000000000')
  reportError('Could Not Save the Provider', new Refused('The endpoint is unavailable.', 'req_k3v9x2m7qd4h8w1p'), { userVisible: true })
  await settle()
  expect(toasts.map(({ title, message, requestId }) => ({ title, message, requestId }))).toEqual([
    { title: 'Could Not Save the Provider', message: 'The endpoint is unavailable.', requestId: 'req_k3v9x2m7qd4h8w1p' },
  ])
  expect(reports).toEqual([])
  expect(shownError(new Refused('Busy', 'req_a'))).toEqual({ message: 'Busy', requestId: 'req_a' })
})

test('the page’s own failure is reported with where it failed, and its toast names the report once it is in', async () => {
  const reports = reporter('req_pagereport00001')
  reportError('Could Not Read the Plugin’s Data', new Error('Invalid server response'), {
    userVisible: true,
    detail: '✖ Invalid input: expected string, received number\n  → at sections[0].title',
  })
  expect(toasts[0]!.requestId).toBeNull()
  await settle()
  expect(reports).toEqual([{
    title: 'Could Not Read the Plugin’s Data',
    message: 'Invalid server response',
    detail: '✖ Invalid input: expected string, received number\n  → at sections[0].title',
  }])
  expect(toasts[0]!.requestId).toBe('req_pagereport00001')
})

test('a report the backend refused leaves the toast without an ID, and an expected failure is not reported', async () => {
  const reports = reporter(null)
  reportError('Could Not Open the File', new Error('handler threw'), { userVisible: true })
  reportError('Could Not Copy', new Error('denied'), { expected: true })
  await settle()
  expect(reports.map((report) => report.title)).toEqual(['Could Not Open the File'])
  // Without a detail of its own, the report says where by the error's stack.
  expect(reports[0]!.detail).toContain('errors.test.ts')
  expect(toasts.map((toast) => toast.requestId)).toEqual([null])
})

test('a report keeps an address’s path and drops its query', async () => {
  const reports = reporter('req_report0000000002')
  reportError('Could Not Open the File', new Error('GET https://demi.example/api/fs/raw?path=/home/ana/.env&token=abc failed; /api/fs/file?path=secret.txt too'))
  await settle()
  expect(reports[0]!.message).toBe('GET https://demi.example/api/fs/raw failed; /api/fs/file too')
})

test('a refusal the user corrects in the form names no request and says the page’s words; any other failure names its request', () => {
  const corrections = { invalid_credentials: 'That is not your current password.' }
  class Coded extends Refused {
    constructor(readonly code: string, message: string) {
      super(message, 'req_k3v9x2m7qd4h8w1p')
    }
  }
  expect(formError(new Coded('invalid_credentials', 'Current password is wrong'), corrections))
    .toEqual({ message: 'That is not your current password.', requestId: null })
  expect(formError(new Coded('internal_error', 'The store failed'), corrections))
    .toEqual({ message: 'The store failed', requestId: 'req_k3v9x2m7qd4h8w1p' })
})

/** A refusal with its code and the values its text needs, as an API error carries them. */
class Worded extends Refused {
  constructor(readonly code: string, message: string, readonly params: Record<string, unknown> = {}) {
    super(message, 'req_k3v9x2m7qd4h8w1p')
  }
}

// The email form's refusal of a taken address: the catalogue's words, which
// the form gives no words of its own for, and no request, since the user
// corrects it there.
test('a refusal the form leaves to the catalogue says its words and names no request', () => {
  const shown = formError(new Worded('email_taken', 'The email address is taken'), { email_taken: null })
  expect(shown).toEqual({ message: 'Another account uses this email address.', requestId: null })
})

// A failure keeps what it was, and the page words it as it shows it: a
// language chosen while it shows rewords it, inline and in its toast
// (`localization.md` § Text from the backend).
test('an error shown before the language changes is reworded in the new language, inline and in its toast', () => {
  const refusal = new Worded('saved_full', 'The conversation keeps 200 messages saved for later', { limit: 200 })
  try {
    const inline = reactive(shownError(refusal))
    const said = computed(() => inline.message)
    reportError('Could Not Save the Message', refusal, { userVisible: true, expected: true })
    const toast = computed(() => toasts[0]!.message)
    expect([said.value, toast.value]).toEqual([
      'A conversation keeps up to 200 messages saved for later.',
      'A conversation keeps up to 200 messages saved for later.',
    ])
    createPageI18n('ja')
    expect([said.value, toast.value]).toEqual([
      '1 つの会話で後で送信するために保存できるメッセージは 200 件までです。',
      '1 つの会話で後で送信するために保存できるメッセージは 200 件までです。',
    ])
  } finally {
    createPageI18n('en')
  }
})

// A failure the page found itself carries the code its words are looked up
// by, and an answer a proxy gave, which has none, is worded from its status.
test('a coded page failure and a codeless answer are worded in the page’s language', () => {
  const lost = coded(new TypeError('Upload connection failed.'), 'upload_connection_failed')
  const proxied = Object.assign(new Error('Request failed (502).'), { code: null, status: 502 })
  try {
    expect([errorMessage(lost), errorMessage(proxied)]).toEqual(['The upload lost its connection.', 'The request failed (502).'])
    createPageI18n('zh-Hans')
    expect([errorMessage(lost), errorMessage(proxied)]).toEqual(['上传连接已断开。', '请求失败（502）。'])
  } finally {
    createPageI18n('en')
  }
})
