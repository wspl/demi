import { afterEach, expect, test } from 'bun:test'
import { createPageI18n } from '../../i18n/text'
import type { Language } from '../../i18n/languages'
import { fileBar } from '../file-bar'
import { entryKind, linkDescription } from '../format'
import { gitMark } from '../git-status'
import { t } from '../locales/text'

// What the file area says in each language (`localization.md`): its words
// come from the catalogue, a count takes the language's plural forms, and
// VS Code's own words mark a file's git status. A table; no I/O.
afterEach(() => {
  createPageI18n('en')
})

test.each<{ language: Language; lines: string; kind: string; marked: string; link: string; uploads: string }>([
  {
    language: 'en',
    lines: 'TypeScript · 1 line · 4 B',
    kind: 'TS file',
    marked: 'Conflict: deleted by them',
    link: 'Symbolic link to /tmp/gone.log, which doesn’t exist',
    uploads: '1 file',
  },
  {
    language: 'zh-Hans',
    lines: 'TypeScript · 1 行 · 4 B',
    kind: 'TS 文件',
    marked: '冲突：对方已删除',
    link: '指向 /tmp/gone.log 的符号链接，目标不存在',
    uploads: '1 个文件',
  },
  {
    language: 'ja',
    lines: 'TypeScript · 1 行 · 4 B',
    kind: 'TS ファイル',
    marked: '競合: 他者が削除',
    link: '/tmp/gone.log へのシンボリック リンク。リンク先は存在しません',
    uploads: '1 個のファイル',
  },
  {
    language: 'ko',
    lines: 'TypeScript · 1줄 · 4 B',
    kind: 'TS 파일',
    marked: '충돌: 타인이 삭제',
    link: '/tmp/gone.log에 대한 심볼 링크이며 대상이 없습니다',
    uploads: '파일 1개',
  },
])('a $language page words the file area in $language', ({ language, lines, kind, marked, link, uploads }) => {
  createPageI18n(language)
  expect(fileBar('/w/a.ts', { as: 'text', text: 'a=1\n' }).label).toBe(lines)
  expect(entryKind('a.ts', false)).toBe(kind)
  expect(gitMark('UD')?.text).toBe(marked)
  expect(linkDescription({ target: '/tmp/gone.log', broken: { kind: 'missing' } })).toBe(link)
  expect(t('uploads.files', { count: 1 }, 1)).toBe(uploads)
})

test('English counts take the singular for one and the plural otherwise', () => {
  expect(t('uploads.files', { count: 1 }, 1)).toBe('1 file')
  expect(t('uploads.files', { count: 40 }, 40)).toBe('40 files')
})
