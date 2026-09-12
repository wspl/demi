import { expect, test } from 'bun:test'
import { configuredModelSchema, modelListFromConfiguredModels } from '../configured-models'

test('configured catalogs preserve explicit null defaults and reject missing or duplicate identities', () => {
  const models = [{ id: 'm', contextWindow: 100 }]
  expect(modelListFromConfiguredModels(models, { providerId: 'p' }).defaultModelId).toBe('m')
  expect(modelListFromConfiguredModels(models, { providerId: 'p', defaultModelId: null }).defaultModelId).toBeNull()
  expect(() => modelListFromConfiguredModels(models, { providerId: 'p', defaultModelId: 'missing' })).toThrow()
  expect(() => modelListFromConfiguredModels([...models, ...models], { providerId: 'p' })).toThrow()
  expect(modelListFromConfiguredModels([], { providerId: 'p' }).defaultModelId).toBeNull()
})

test('configured model schemas reject wrong types and do not truncate token limits', () => {
  for (const model of [
    {}, { id: '', contextWindow: 100 }, { id: 'm', contextWindow: '100' },
    { id: 'm', contextWindow: 1.5 }, { id: 'm', contextWindow: 0 },
    { id: 'm', contextWindow: 100, outputLimit: -1 },
    { id: 'm', contextWindow: 100, supportsTools: 'true' },
    { id: 'm', contextWindow: 100, supportedThinkingEfforts: {} },
    { id: 'm', contextWindow: 100, serviceTiers: [{ id: 'fast', label: 'Fast', fast: 'yes' }] },
    { id: 'm', contextWindow: 100, unknown: true },
  ]) {
    expect(configuredModelSchema.safeParse(model).success).toBe(false)
  }
})

test('configured catalog projection separates caller arrays and keeps missing capabilities unknown', () => {
  const models = [{
    id: 'm', contextWindow: 100, supportedThinkingEfforts: ['high'],
    serviceTiers: [{ id: 'priority', label: 'Fast', fast: true }],
  }]
  const result = modelListFromConfiguredModels(models, { providerId: 'p' })
  expect(result.models[0]).toMatchObject({
    displayName: 'm', outputLimit: null, supportsTools: null, supportsAttachments: null,
    supportsVideo: null, supportsReasoning: null, defaultThinkingEffort: null,
  })
  result.models[0]!.supportedThinkingEfforts!.push('low')
  result.models[0]!.serviceTiers![0]!.label = 'Modified'
  expect(models[0]!.supportedThinkingEfforts).toEqual(['high'])
  expect(models[0]!.serviceTiers[0]!.label).toBe('Fast')
})
