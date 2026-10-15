import { afterAll, expect, test } from 'bun:test'
import { createPageI18n } from '../../i18n/text'
import type { Language } from '../../i18n/languages'
import { receiptRow } from '../agent-receipts'
import en from '../locales/en.json'
import ja from '../locales/ja.json'
import ko from '../locales/ko.json'
import zhHans from '../locales/zh-Hans.json'
import zhHant from '../locales/zh-Hant.json'
import { t } from '../locales/text'

// Cost: pure, a few milliseconds for the file.

const languages = { 'zh-Hans': zhHans, 'zh-Hant': zhHant, ja, ko } as const

/** Every text of a catalogue by its key, `transcript.work.ran`. */
function texts(tree: object, prefix = ''): [string, string][] {
  return Object.entries(tree).flatMap(([key, value]) =>
    typeof value === 'string' ? [[`${prefix}${key}`, value] as [string, string]] : texts(value, `${prefix}${key}.`))
}

/** The values a text is filled with, `{count}` and `{span}`. */
function values(text: string): string[] {
  return [...new Set([...text.matchAll(/\{(\w+)\}/g)].map((found) => found[1]!))].sort()
}

// The catalogues are typed by their keys, not by what each text is filled with: a translation that
// drops {span} or renames {count} shows a hole or the name itself in front of a user.
test('every language fills each text with the values the English one does', () => {
  const english = new Map(texts(en))
  for (const [language, catalogue] of Object.entries(languages)) {
    for (const [key, text] of texts(catalogue)) {
      const source = english.get(key)!
      // A count is the plural form's own, so a language with one form may leave it out.
      const expected = values(source).filter((name) => name !== 'count')
      expect({ language, key, values: values(text).filter((name) => name !== 'count') })
        .toEqual({ language, key, values: expected })
    }
  }
})

test('no text holds a character vue-i18n reads as syntax outside a value', () => {
  for (const [language, catalogue] of [['en', en] as const, ...Object.entries(languages)]) {
    for (const [key, text] of texts(catalogue)) {
      expect({ language, key, syntax: text.replace(/\{\w+\}/g, '').match(/[@$]/)?.[0] ?? null })
        .toEqual({ language, key, syntax: null })
    }
  }
})

afterAll(() => createPageI18n('en'))

test('a count takes its language’s plural forms', () => {
  const words = (language: Language) => {
    createPageI18n(language)
    return [1, 2].map((count) => t('subagents.chip', { count }, count))
  }
  expect(words('en')).toEqual(['1 Agent', '2 Agents'])
  expect(words('ja')).toEqual(['1 個のエージェント', '2 個のエージェント'])
})

test('a notice that names an agent inside its sentence is split where the language puts the agent', () => {
  const fromMain = {
    id: 'm1',
    sender: { id: 'conversation-1', number: 0, description: '', round: 1 },
    recipientId: 'ag-3',
    timestamp: '2026-10-11T12:00:00.000Z',
    content: 'Hi.',
    event: { type: 'message' as const },
  }
  createPageI18n('en')
  expect(receiptRow(fromMain)).toMatchObject({ notice: 'Message from ', title: 'main agent', after: '' })
  createPageI18n('ja')
  expect(receiptRow(fromMain)).toMatchObject({ notice: '', title: 'メインエージェント', after: 'からのメッセージ' })
})
