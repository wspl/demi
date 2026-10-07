import { expect, test } from 'bun:test'
import { addressUrl } from '../address'

test('a local name, an IP address or a host with a port opens over http, as in Chrome', () => {
  expect(addressUrl('localhost:8000/orders')).toBe('http://localhost:8000/orders')
  expect(addressUrl('localhost')).toBe('http://localhost/')
  expect(addressUrl('app.localhost:3000')).toBe('http://app.localhost:3000/')
  expect(addressUrl('127.0.0.1:5173/?a=1')).toBe('http://127.0.0.1:5173/?a=1')
  expect(addressUrl('192.168.1.20')).toBe('http://192.168.1.20/')
  expect(addressUrl('[::1]:8080')).toBe('http://[::1]:8080/')
  expect(addressUrl('devbox:8080/health')).toBe('http://devbox:8080/health')
})

test('other text with a dot and no space opens over https', () => {
  expect(addressUrl('example.com')).toBe('https://example.com/')
  expect(addressUrl('  docs.rs/tokio  ')).toBe('https://docs.rs/tokio')
})

test('an address with its scheme opens as it is', () => {
  expect(addressUrl('http://example.com/a')).toBe('http://example.com/a')
  expect(addressUrl('about:blank')).toBe('about:blank')
  expect(addressUrl('file:///tmp/a.html')).toBe('file:///tmp/a.html')
})

test('words, or a single word, are a Google search', () => {
  expect(addressUrl('rust async book')).toBe('https://www.google.com/search?q=rust%20async%20book')
  expect(addressUrl('vitest')).toBe('https://www.google.com/search?q=vitest')
  expect(addressUrl('how do I use example.com')).toBe('https://www.google.com/search?q=how%20do%20I%20use%20example.com')
})

test('nothing typed opens nothing', () => {
  expect(addressUrl('   ')).toBeNull()
})
