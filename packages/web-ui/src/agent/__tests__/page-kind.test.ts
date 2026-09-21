import { expect, test } from 'bun:test'
import { exposePageTab, loadPageAddress, pageTabDataSchema, pageTabTitle } from '../panel-kinds/page-data'

test('an expose opens a tab on its URL, named by the address it exposes', () => {
  const data = exposePageTab({ url: 'https://abc.expose.demi.example/', address: '127.0.0.1:5173' })
  expect(pageTabDataSchema.parse(data)).toEqual(data)
  expect(pageTabTitle(data)).toBe('127.0.0.1:5173')
  // Submitting another address makes it an ordinary page, named by its host.
  const next = loadPageAddress(data, 'example.com')
  expect(next).toEqual({ url: 'https://example.com/', expose: null })
  expect(pageTabTitle(next)).toBe('example.com')
})

test('submitting the address loads http and https, tries a bare host as https, and ignores the rest', () => {
  const data = exposePageTab({ url: 'https://abc.expose.demi.example/', address: '127.0.0.1:5173' })
  expect(loadPageAddress(data, 'http://localhost:5173/app').url).toBe('http://localhost:5173/app')
  expect(loadPageAddress(data, ' example.com ').url).toBe('https://example.com/')
  for (const address of ['', '   ', 'javascript://alert(1)', 'file:///etc/passwd', 'not a url']) {
    expect(loadPageAddress(data, address)).toBe(data)
  }
})
