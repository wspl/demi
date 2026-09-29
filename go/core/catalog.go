package core

//demi:wire
type ProviderModelList struct {
	Models          []ProviderModel `json:"models"`
	DefaultModelID  *string         `json:"defaultModelId" check:"nullable"`
	Warnings        []string        `json:"warnings"`
	SourceFetchedAt Timestamp       `json:"sourceFetchedAt" check:"func=Validate"`
	Stale           bool            `json:"stale"`
}

//demi:wire
type ProviderModel struct {
	ID                       string           `json:"id" check:"chars=1.."`
	DisplayName              string           `json:"displayName"`
	Description              *string          `json:"description" check:"nullable"`
	ContextWindow            *uint32          `json:"contextWindow" check:"nullable"`
	OutputLimit              *uint32          `json:"outputLimit" check:"nullable,range=1.."`
	SupportsTools            *bool            `json:"supportsTools" check:"nullable"`
	SupportsAttachments      *bool            `json:"supportsAttachments" check:"nullable"`
	SupportsVideo            *bool            `json:"supportsVideo" check:"nullable"`
	AcceptedExtensions       *[]FileExtension `json:"acceptedExtensions" check:"nullable"`
	SupportsReasoning        *bool            `json:"supportsReasoning" check:"nullable"`
	SupportedThinkingEfforts *[]string        `json:"supportedThinkingEfforts" check:"nullable"`
	DefaultThinkingEffort    *string          `json:"defaultThinkingEffort" check:"nullable"`
	CanDisableThinking       *bool            `json:"canDisableThinking" check:"nullable"`
	ServiceTiers             []ServiceTier    `json:"serviceTiers"`
	DefaultServiceTierID     *string          `json:"defaultServiceTierId" check:"nullable"`
	Cost                     *ModelCost       `json:"cost" check:"nullable"`
}

//demi:wire
type ServiceTier struct {
	ID          string  `json:"id" check:"chars=1.."`
	Label       string  `json:"label"`
	Description *string `json:"description" check:"nullable"`
	Fast        bool    `json:"fast"`
}

//demi:wire
type ModelCost struct {
	Input      *float64 `json:"input" check:"nullable"`
	Output     *float64 `json:"output" check:"nullable"`
	CacheRead  *float64 `json:"cacheRead" check:"nullable"`
	CacheWrite *float64 `json:"cacheWrite" check:"nullable"`
}
