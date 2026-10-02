package core

// ProviderModelList is one catalog read, including freshness and warnings.
// +demi:root direction=receive output=protocol
type ProviderModelList struct {
	Models []ProviderModel `json:"models"`
	// +demi:nullable
	DefaultModelID  *string   `json:"defaultModelId"`
	Warnings        []string  `json:"warnings"`
	SourceFetchedAt Timestamp `json:"sourceFetchedAt"`
	Stale           bool      `json:"stale"`
}

// ProviderModel holds the portable facts a catalog states about one model.
type ProviderModel struct {
	// +demi:length chars min=1
	ID          string `json:"id"`
	DisplayName string `json:"displayName"`
	// +demi:nullable
	Description *string `json:"description"`
	// +demi:nullable
	ContextWindow *uint32 `json:"contextWindow"`
	// +demi:nullable
	// +demi:range min=1
	OutputLimit *uint32 `json:"outputLimit"`
	// +demi:nullable
	SupportsTools *bool `json:"supportsTools"`
	// +demi:nullable
	SupportsAttachments *bool `json:"supportsAttachments"`
	// +demi:nullable
	SupportsVideo *bool `json:"supportsVideo"`
	// +demi:nullable
	AcceptedExtensions *[]FileExtension `json:"acceptedExtensions"`
	// +demi:nullable
	SupportsReasoning *bool `json:"supportsReasoning"`
	// +demi:nullable
	SupportedThinkingEfforts *[]string `json:"supportedThinkingEfforts"`
	// +demi:nullable
	DefaultThinkingEffort *string `json:"defaultThinkingEffort"`
	// +demi:nullable
	CanDisableThinking *bool         `json:"canDisableThinking"`
	ServiceTiers       []ServiceTier `json:"serviceTiers"`
	// +demi:nullable
	DefaultServiceTierID *string `json:"defaultServiceTierId"`
	// +demi:nullable
	Cost *ModelCost `json:"cost"`
}

// ServiceTier names a service tier offered by a model.
type ServiceTier struct {
	// +demi:length chars min=1
	ID    string `json:"id"`
	Label string `json:"label"`
	// +demi:nullable
	Description *string `json:"description"`
	Fast        bool    `json:"fast"`
}

// ModelCost holds reported dollar prices per million tokens.
type ModelCost struct {
	// +demi:nullable
	Input *float64 `json:"input"`
	// +demi:nullable
	Output *float64 `json:"output"`
	// +demi:nullable
	CacheRead *float64 `json:"cacheRead"`
	// +demi:nullable
	CacheWrite *float64 `json:"cacheWrite"`
}
