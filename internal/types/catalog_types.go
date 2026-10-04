package types

// An entry's catalog as one read of its source returned it. The backend
// keeps it in its catalog cache and reads it back from storage, so it
// refuses unknown fields.
// +demi:root direction=receive output=protocol
type ProviderModelList struct {
	Models []ProviderModel `json:"models"`
	// The model a new selection starts with; null when the source names
	// none.
	// +demi:nullable
	DefaultModelID *string `json:"defaultModelId"`
	// What went wrong while the catalog was read, such as a refresh that
	// failed and left an older copy.
	Warnings []string `json:"warnings"`
	// When the source was last downloaded; the Unix epoch for a list that
	// was never fetched, such as one built into a provider.
	SourceFetchedAt Timestamp `json:"sourceFetchedAt"`
	// Whether this is a copy kept after a failed refresh.
	Stale bool `json:"stale"`
}

// One model of a catalog, as its source describes it.
type ProviderModel struct {
	// +demi:length chars min=1
	ID          string `json:"id"`
	DisplayName string `json:"displayName"`
	// +demi:nullable
	Description *string `json:"description"`
	// Tokens.
	// +demi:nullable
	ContextWindow *uint32 `json:"contextWindow"`
	// The most tokens one request may generate; null when no
	// model-specific limit is known (`models.md` § Output limit).
	// +demi:nullable
	// +demi:range min=1
	OutputLimit *uint32 `json:"outputLimit"`
	// +demi:nullable
	SupportsTools *bool `json:"supportsTools"`
	// Whether the model reads images and documents natively.
	// +demi:nullable
	SupportsAttachments *bool `json:"supportsAttachments"`
	// Whether the model reads video natively.
	// +demi:nullable
	SupportsVideo *bool `json:"supportsVideo"`
	// The exact types the model reads natively, when the source states
	// them: `[]` for none, null when the source does not say
	// (`models.md` § Accepted attachment types).
	// +demi:nullable
	AcceptedExtensions *[]FileExtension `json:"acceptedExtensions"`
	// +demi:nullable
	SupportsReasoning *bool `json:"supportsReasoning"`
	// The thinking efforts the model offers, in the vendor's words.
	// +demi:nullable
	SupportedThinkingEfforts *[]string `json:"supportedThinkingEfforts"`
	// +demi:nullable
	DefaultThinkingEffort *string `json:"defaultThinkingEffort"`
	// Whether thinking can be turned off entirely; a transport that only
	// levels thinking says no.
	// +demi:nullable
	CanDisableThinking *bool `json:"canDisableThinking"`
	// The service tiers the model offers; empty for none.
	ServiceTiers []ServiceTier `json:"serviceTiers"`
	// +demi:nullable
	DefaultServiceTierID *string `json:"defaultServiceTierId"`
	// Prices as the source reports them.
	// +demi:nullable
	Cost *ModelCost `json:"cost"`
}

// A service tier a model offers. The product's Fast switch selects the tier
// marked `fast` and nothing else.
type ServiceTier struct {
	// +demi:length chars min=1
	ID    string `json:"id"`
	Label string `json:"label"`
	// +demi:nullable
	Description *string `json:"description"`
	Fast        bool    `json:"fast"`
}

// A model's prices in dollars per million tokens; null for a price the
// source does not report.
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
