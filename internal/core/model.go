package core

// FileExtension is a file type a model can read natively (`models.md` § Accepted attachment
// types). Extensions omit the dot; `jpg` and `jpeg` are one format.
// +demi:enum png jpg jpeg gif webp pdf mp4 mov webm m4v
type FileExtension string

// Model is a model with its catalog facts, as a selection records it.
type Model struct {
	// +demi:length min=1
	ID   string `json:"id"`
	Name string `json:"name"`
	// Tokens; zero when the catalog does not know it.
	ContextWindow uint32 `json:"contextWindow"`
	// +demi:nullable
	InputLimit *uint32 `json:"inputLimit"`
	// The most tokens one request may generate: a positive whole number, or
	// null when no model-specific limit is known.
	// +demi:nullable
	// +demi:range min=1
	OutputLimit *uint32              `json:"outputLimit"`
	Thinking    []ThinkingCapability `json:"thinking"`
	// The types the model reads natively: `[]` for none, null when unknown.
	// +demi:nullable
	AcceptedExtensions *[]FileExtension `json:"acceptedExtensions"`
}

// ModelSelection is the model a conversation infers with and how: the provider entry, the
// model with its facts, the thinking setting and the service tier. Every
// block records the selection that was current when it was written.
type ModelSelection struct {
	// +demi:length min=1
	ProviderID string `json:"providerId"`
	Model      Model  `json:"model"`
	// +demi:nullable
	Thinking ThinkingConfig `json:"thinking"`
	// The service tier, such as the one the catalog marks Fast; null for the
	// vendor's default.
	// +demi:nullable
	// +demi:length min=1
	ServiceTierID *string `json:"serviceTierId"`
}

// ThinkingSummary records whether a reasoning summary is asked for, and how detailed.
// +demi:enum auto concise detailed off on
type ThinkingSummary string

// ThinkingCapability is one way a model can think, as its catalog offers it. Effort levels are the
// vendor's words, such as `low` or `xhigh`.
// +demi:union tag=type
//
//sumtype:decl
type ThinkingCapability interface{ isThinkingCapability() }

// AdaptiveCapability is a transcript contract value.
// +demi:variant ThinkingCapability adaptive
type AdaptiveCapability struct {
	Efforts []string `json:"efforts"`
	// +demi:nullable
	DefaultEffort *string `json:"defaultEffort"`
}

// BudgetCapability is a transcript contract value.
// +demi:variant ThinkingCapability budget
type BudgetCapability struct {
	// +demi:nullable
	MinBudgetTokens *uint32 `json:"minBudgetTokens"`
	// +demi:nullable
	MaxBudgetTokens *uint32 `json:"maxBudgetTokens"`
	// +demi:nullable
	DefaultBudgetTokens *uint32 `json:"defaultBudgetTokens"`
}

// EffortCapability is a transcript contract value.
// +demi:variant ThinkingCapability effort
type EffortCapability struct {
	Efforts []string `json:"efforts"`
	// +demi:nullable
	DefaultEffort *string           `json:"defaultEffort"`
	Summaries     []ThinkingSummary `json:"summaries"`
	// +demi:nullable
	DefaultSummary *ThinkingSummary `json:"defaultSummary"`
}

// DisabledCapability represents thinking can be turned off.
// +demi:variant ThinkingCapability disabled
type DisabledCapability struct{}

// ThinkingConfig is the thinking setting a selection makes, which each provider maps onto its
// vendor's option.
// +demi:union tag=type
//
//sumtype:decl
type ThinkingConfig interface{ isThinkingConfig() }

// AdaptiveConfig is a transcript contract value.
// +demi:variant ThinkingConfig adaptive
type AdaptiveConfig struct {
	Effort string `json:"effort"`
}

// BudgetConfig is a transcript contract value.
// +demi:variant ThinkingConfig budget
type BudgetConfig struct {
	BudgetTokens uint32 `json:"budgetTokens"`
}

// EffortConfig is a transcript contract value.
// +demi:variant ThinkingConfig effort
type EffortConfig struct {
	Effort string `json:"effort"`
	// +demi:nullable
	Summary *ThinkingSummary `json:"summary"`
}

// DisabledConfig is a transcript contract value.
// +demi:variant ThinkingConfig disabled
type DisabledConfig struct{}

// TokenUsage is the tokens one completed request used, as the provider reported them.
type TokenUsage struct {
	// +demi:range max=9007199254740991
	InputTokens uint64 `json:"inputTokens"`
	// +demi:range max=9007199254740991
	OutputTokens uint64 `json:"outputTokens"`
	// +demi:range max=9007199254740991
	CacheReadTokens uint64 `json:"cacheReadTokens"`
	// +demi:range max=9007199254740991
	CacheWriteTokens uint64 `json:"cacheWriteTokens"`
}
