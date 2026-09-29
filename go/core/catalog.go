package core

// An entry's catalog as one read of its source returned it. The backend
// keeps it in its catalog cache and reads it back from storage, so it
// refuses unknown fields.
//
//demi:wire
type ProviderModelList struct {
	Models []ProviderModel `json:"models"`
	// The model a new selection starts with; null when the source names
	// none.
	DefaultModelID *string `json:"defaultModelId" check:"nullable"`
	// What went wrong while the catalog was read, such as a refresh that
	// failed and left an older copy.
	Warnings []string `json:"warnings"`
	// When the source was last downloaded; the Unix epoch for a list that
	// was never fetched, such as one built into a provider.
	SourceFetchedAt Timestamp `json:"sourceFetchedAt" check:"func=Validate"`
	// Whether this is a copy kept after a failed refresh.
	Stale bool `json:"stale"`
}

// One model of a catalog, as its source describes it.
//
//demi:wire
type ProviderModel struct {
	ID          string  `json:"id" check:"chars=1.."`
	DisplayName string  `json:"displayName"`
	Description *string `json:"description" check:"nullable"`
	// Tokens.
	ContextWindow *uint32 `json:"contextWindow" check:"nullable"`
	// The most tokens one request may generate; null when no
	// model-specific limit is known (`models.md` § Output limit).
	OutputLimit   *uint32 `json:"outputLimit" check:"nullable,range=1.."`
	SupportsTools *bool   `json:"supportsTools" check:"nullable"`
	// Whether the model reads images and documents natively.
	SupportsAttachments *bool `json:"supportsAttachments" check:"nullable"`
	// Whether the model reads video natively.
	SupportsVideo *bool `json:"supportsVideo" check:"nullable"`
	// The exact types the model reads natively, when the source states
	// them: `[]` for none, null when the source does not say
	// (`models.md` § Accepted attachment types).
	AcceptedExtensions *[]FileExtension `json:"acceptedExtensions" check:"nullable"`
	SupportsReasoning  *bool            `json:"supportsReasoning" check:"nullable"`
	// The thinking efforts the model offers, in the vendor's words.
	SupportedThinkingEfforts *[]string `json:"supportedThinkingEfforts" check:"nullable"`
	DefaultThinkingEffort    *string   `json:"defaultThinkingEffort" check:"nullable"`
	// Whether thinking can be turned off entirely; a transport that only
	// levels thinking says no.
	CanDisableThinking *bool `json:"canDisableThinking" check:"nullable"`
	// The service tiers the model offers; empty for none.
	ServiceTiers         []ServiceTier `json:"serviceTiers"`
	DefaultServiceTierID *string       `json:"defaultServiceTierId" check:"nullable"`
	// Prices as the source reports them.
	Cost *ModelCost `json:"cost" check:"nullable"`
}

// A service tier a model offers. The product's Fast switch selects the tier
// marked `fast` and nothing else.
//
//demi:wire
type ServiceTier struct {
	ID          string  `json:"id" check:"chars=1.."`
	Label       string  `json:"label"`
	Description *string `json:"description" check:"nullable"`
	Fast        bool    `json:"fast"`
}

// A model's prices in dollars per million tokens; null for a price the
// source does not report.
//
//demi:wire
type ModelCost struct {
	Input      *float64 `json:"input" check:"nullable"`
	Output     *float64 `json:"output" check:"nullable"`
	CacheRead  *float64 `json:"cacheRead" check:"nullable"`
	CacheWrite *float64 `json:"cacheWrite" check:"nullable"`
}
