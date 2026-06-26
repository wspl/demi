import { describe, expect, it } from 'bun:test'
import { DEFAULT_ATTACHMENT_EXTENSIONS, modelSelectionFromCatalog, thinkingCapabilitiesFromProviderModel } from '../index'
import type { ProviderModel } from '../types'

function providerModel(overrides: Partial<ProviderModel> = {}): ProviderModel {
  return {
    providerId: 'p',
    id: 'm-1',
    displayName: 'Model One',
    contextWindow: 200_000,
    outputLimit: null,
    supportsTools: null,
    supportsAttachments: null,
    supportsReasoning: null,
    supportedThinkingEfforts: null,
    defaultThinkingEffort: null,
    sourceFetchedAt: '2026-06-25T00:00:00.000Z',
    stale: false,
    ...overrides,
  }
}

describe('modelSelectionFromCatalog', () => {
  it('maps a catalog model into a selection', () => {
    const selection = modelSelectionFromCatalog('p', providerModel({ supportsAttachments: true }))
    expect(selection.providerId).toBe('p')
    expect(selection.model.id).toBe('m-1')
    expect(selection.model.name).toBe('Model One')
    expect(selection.model.contextWindow).toBe(200_000)
    expect(selection.model.inputLimit).toBeNull()
    expect(selection.model.acceptedExtensions).toEqual([...DEFAULT_ATTACHMENT_EXTENSIONS])
    expect(selection.thinking).toBeNull()
    expect(selection.serviceTierId).toBeNull()
  })

  it('omits attachment extensions when the model does not support them', () => {
    const selection = modelSelectionFromCatalog('p', providerModel({ supportsAttachments: false }))
    expect(selection.model.acceptedExtensions).toEqual([])
  })

  it('respects an attachment-extension override', () => {
    const selection = modelSelectionFromCatalog('p', providerModel({ supportsAttachments: true }), {
      acceptedExtensions: ['png'],
    })
    expect(selection.model.acceptedExtensions).toEqual(['png'])
  })

  it('falls back to id/options when the catalog entry is absent', () => {
    const selection = modelSelectionFromCatalog('p', null, { modelId: 'pending', fallbackName: 'Pending' })
    expect(selection.model.id).toBe('pending')
    expect(selection.model.name).toBe('Pending')
    expect(selection.model.contextWindow).toBe(0)
    expect(selection.model.acceptedExtensions).toEqual([])
    expect(selection.model.thinking).toEqual([])
  })

  it('passes through thinking config and service tier', () => {
    const selection = modelSelectionFromCatalog('p', providerModel(), {
      thinking: { type: 'effort', effort: 'high', summary: null },
      serviceTierId: 'priority',
    })
    expect(selection.thinking).toEqual({ type: 'effort', effort: 'high', summary: null })
    expect(selection.serviceTierId).toBe('priority')
  })
})

describe('thinkingCapabilitiesFromProviderModel', () => {
  it('returns nothing without a model', () => {
    expect(thinkingCapabilitiesFromProviderModel(null)).toEqual([])
  })

  it('reports disabled when reasoning is unsupported', () => {
    expect(thinkingCapabilitiesFromProviderModel(providerModel({ supportsReasoning: false }))).toEqual([
      { type: 'disabled' },
    ])
  })

  it('exposes supported efforts', () => {
    const caps = thinkingCapabilitiesFromProviderModel(
      providerModel({ supportedThinkingEfforts: ['low', 'high'] }),
    )
    expect(caps).toEqual([
      {
        type: 'effort',
        efforts: ['low', 'high'],
        defaultEffort: null,
        summaries: ['auto', 'concise', 'detailed', 'off', 'on'],
        defaultSummary: null,
      },
    ])
  })
})
