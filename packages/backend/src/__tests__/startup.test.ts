import { expect, test } from 'bun:test'
import { backendStartupFromEnv } from '../startup'

test('backend startup validates environment before starting services', () => {
  expect(
    backendStartupFromEnv(
      { DEMI_INSTANCE_MODE: 'isolated', PATH: '/bin' },
      '/data',
    ),
  ).toMatchObject({ dataDir: '/data', port: 3271, mode: 'isolated' })
  for (const port of [
    '',
    ' ',
    '0',
    '-1',
    '65536',
    'NaN',
    '1.5',
    '1e3',
    '0x100',
  ]) {
    expect(() =>
      backendStartupFromEnv(
        {
          DEMI_INSTANCE_MODE: 'shared',
          DEMI_BACKEND_PORT: port,
        },
        '/data',
      ),
    ).toThrow('DEMI_BACKEND_PORT')
  }
  for (const key of [
    'DEMI_BACKEND_DATA',
    'DEMI_WEB_DIRECTORY',
    'DEMI_MACHINES_SOCKET',
  ]) {
    expect(() =>
      backendStartupFromEnv(
        {
          DEMI_INSTANCE_MODE: 'isolated',
          [key]: ' ',
        },
        '/data',
      ),
    ).toThrow(key)
  }
  expect(() => backendStartupFromEnv({}, '/data')).toThrow('DEMI_INSTANCE_MODE')
  expect(() =>
    backendStartupFromEnv({ DEMI_INSTANCE_MODE: 'other' }, '/data'),
  ).toThrow('DEMI_INSTANCE_MODE')
  expect(() =>
    backendStartupFromEnv(
      {
        DEMI_INSTANCE_MODE: 'shared',
        DEMI_MACHINES_SOCKET: '/machines.sock',
      },
      '/data',
    ),
  ).toThrow('DEMI_BACKEND_PUBLIC_URL')
  expect(() =>
    backendStartupFromEnv(
      {
        DEMI_INSTANCE_MODE: 'shared',
        DEMI_BACKEND_PUBLIC_URL: 'file:///backend',
      },
      '/data',
    ),
  ).toThrow('DEMI_BACKEND_PUBLIC_URL')
  expect(
    backendStartupFromEnv(
      {
        DEMI_INSTANCE_MODE: 'shared',
        DEMI_BACKEND_PORT: '65535',
        DEMI_MACHINES_SOCKET: '/machines.sock',
        DEMI_BACKEND_PUBLIC_URL: 'https://backend.test',
      },
      '/data',
    ),
  ).toMatchObject({ port: 65535, machinesSocket: '/machines.sock' })
})
