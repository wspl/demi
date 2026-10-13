import { expect, test } from 'bun:test'
import { deviceSystem, unlockSentence } from '../device-unlock'

// Cost: pure functions; under a millisecond.
const SAFARI_IPHONE = 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1'
const SAFARI_IPAD = 'Mozilla/5.0 (iPad; CPU OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1'
const SAFARI_MAC = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15'
const EDGE_WINDOWS = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36 Edg/141.0.0.0'
const CHROME_ANDROID = 'Mozilla/5.0 (Linux; Android 15; Pixel 9) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Mobile Safari/537.36'
const FIREFOX_LINUX = 'Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0'
const CHROME_OS = 'Mozilla/5.0 (X11; CrOS x86_64 16093.0.0) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36'

test('the offer names the unlock of the browser’s system', () => {
  const sentence = (userAgent: string, maxTouchPoints = 0) => unlockSentence(deviceSystem({ userAgent, maxTouchPoints }))
  expect(sentence(SAFARI_IPHONE, 5)).toBe('Sign in with Face ID or Touch ID instead of your password.')
  expect(sentence(SAFARI_IPAD, 5)).toBe('Sign in with Face ID or Touch ID instead of your password.')
  // An iPad's Safari asks for the desktop site and says it is a Mac, with a touch screen.
  expect(sentence(SAFARI_MAC, 5)).toBe('Sign in with Face ID or Touch ID instead of your password.')
  expect(sentence(SAFARI_MAC)).toBe('Sign in with Touch ID instead of your password.')
  expect(sentence(EDGE_WINDOWS)).toBe('Sign in with Windows Hello instead of your password.')
  expect(sentence(CHROME_ANDROID, 5)).toBe('Sign in with your fingerprint, face or screen lock instead of your password.')
  expect(sentence(FIREFOX_LINUX)).toBe('Sign in with your device’s screen lock instead of your password.')
  expect(sentence(CHROME_OS)).toBe('Sign in with your device’s screen lock instead of your password.')
  expect(sentence('curl/8.7.1')).toBe('Sign in with your device’s screen lock instead of your password.')
})
