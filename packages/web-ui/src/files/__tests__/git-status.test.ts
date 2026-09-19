import { expect, test } from 'bun:test'
import { gitMark, gitResourceStatus } from '../git-status'

// VS Code's explorer lets a later resource group's mark replace an earlier
// one's: index, untracked, working tree, conflicts.

test('one letter of git marks a file as that group of VS Code does', () => {
  expect(gitResourceStatus('M ')).toBe('index-modified')
  expect(gitResourceStatus('A ')).toBe('index-added')
  expect(gitResourceStatus('D ')).toBe('index-deleted')
  expect(gitResourceStatus('R ')).toBe('index-renamed')
  expect(gitResourceStatus('C ')).toBe('index-copied')
  expect(gitResourceStatus(' M')).toBe('modified')
  expect(gitResourceStatus(' D')).toBe('deleted')
  expect(gitResourceStatus(' A')).toBe('intent-to-add')
  expect(gitResourceStatus(' R')).toBe('intent-to-rename')
  expect(gitResourceStatus(' T')).toBe('type-changed')
  expect(gitResourceStatus('??')).toBe('untracked')
})

test('the working tree outranks the index, and a conflict everything', () => {
  // A staged new file edited again shows M; a staged rename edited again, M too.
  expect(gitResourceStatus('AM')).toBe('modified')
  expect(gitResourceStatus('RM')).toBe('modified')
  expect(gitResourceStatus('MD')).toBe('deleted')
  expect(gitResourceStatus('UU')).toBe('both-modified')
  expect(gitResourceStatus('DD')).toBe('both-deleted')
  expect(gitResourceStatus('AU')).toBe('added-by-us')
  // VS Code files no index resource for a staged type change.
  expect(gitResourceStatus('T ')).toBeNull()
})

test('each status wears VS Code\'s letter, words, color and strike', () => {
  expect(gitMark('??')).toEqual({ letter: 'U', text: 'Untracked', color: 'untracked', strike: false })
  expect(gitMark('A ')).toEqual({ letter: 'A', text: 'Index Added', color: 'added', strike: false })
  expect(gitMark(' D')).toEqual({ letter: 'D', text: 'Deleted', color: 'deleted', strike: true })
  expect(gitMark('UD')).toEqual({ letter: '!', text: 'Conflict: Deleted By Them', color: 'conflicting', strike: true })
  expect(gitMark('T ')).toBeNull()
})
