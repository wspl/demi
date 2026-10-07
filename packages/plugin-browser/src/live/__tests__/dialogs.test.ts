import { expect, test } from 'bun:test'
import { dialogTitle, keyAnswer } from '../dialogs'

test('a page’s dialog says which host asks, as a browser’s does', () => {
  expect(dialogTitle('alert', 'localhost:8000')).toBe('localhost:8000 says')
  expect(dialogTitle('confirm', 'shop.example.com')).toBe('shop.example.com says')
  expect(dialogTitle('prompt', '')).toBe('This page says')
  expect(dialogTitle('beforeunload', 'localhost:8000')).toBe('Leave the page?')
})

test('Enter answers with the default button and Escape cancels, an alert’s Escape closing it', () => {
  expect(keyAnswer('Enter', 'confirm')).toBe(true)
  expect(keyAnswer('Enter', 'prompt')).toBe(true)
  expect(keyAnswer('Escape', 'confirm')).toBe(false)
  expect(keyAnswer('Escape', 'beforeunload')).toBe(false)
  expect(keyAnswer('Escape', 'alert')).toBe(true)
  expect(keyAnswer('a', 'prompt')).toBeNull()
})
