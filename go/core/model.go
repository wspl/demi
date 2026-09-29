package core

// A model with its catalog facts, as a selection records it.
//
//demi:wire
type Model struct {
	ID   string `json:"id" check:"chars=1.."`
	Name string `json:"name"`
	// Tokens; zero when the catalog does not know it.
	ContextWindow uint32  `json:"contextWindow"`
	InputLimit    *uint32 `json:"inputLimit" check:"nullable"`
	// The most tokens one request may generate: a positive whole number, or
	// null when no model-specific limit is known.
	OutputLimit *uint32              `json:"outputLimit" check:"nullable,range=1.."`
	Thinking    []ThinkingCapability `json:"thinking"`
	// The types the model reads natively: `[]` for none, null when unknown.
	AcceptedExtensions *[]FileExtension `json:"acceptedExtensions" check:"nullable"`
}

// The model a conversation infers with and how: the provider entry, the
// model with its facts, the thinking setting and the service tier. Every
// block records the selection that was current when it was written.
//
//demi:wire
type ModelSelection struct {
	ProviderID string          `json:"providerId" check:"chars=1.."`
	Model      Model           `json:"model"`
	Thinking   *ThinkingConfig `json:"thinking" check:"nullable"`
	// The service tier, such as the one the catalog marks Fast; null for the
	// vendor's default.
	ServiceTierID *string `json:"serviceTierId" check:"nullable,chars=1.."`
}

// The tokens one completed request used, as the provider reported them.
//
//demi:wire
type TokenUsage struct {
	InputTokens      uint64 `json:"inputTokens" check:"range=..MaxSafeInteger"`
	OutputTokens     uint64 `json:"outputTokens" check:"range=..MaxSafeInteger"`
	CacheReadTokens  uint64 `json:"cacheReadTokens" check:"range=..MaxSafeInteger"`
	CacheWriteTokens uint64 `json:"cacheWriteTokens" check:"range=..MaxSafeInteger"`
}

// A file type a model can read natively (`models.md` § Accepted attachment
// types). Extensions omit the dot; `jpg` and `jpeg` are one format.
//
//demi:enum
//demi:export
type FileExtension string

const (
	FileExtensionPng  FileExtension = "png"
	FileExtensionJpg  FileExtension = "jpg"
	FileExtensionJpeg FileExtension = "jpeg"
	FileExtensionGif  FileExtension = "gif"
	FileExtensionWebp FileExtension = "webp"
	FileExtensionPdf  FileExtension = "pdf"
	FileExtensionMp4  FileExtension = "mp4"
	FileExtensionMov  FileExtension = "mov"
	FileExtensionWebm FileExtension = "webm"
	FileExtensionM4v  FileExtension = "m4v"
)

// Whether a reasoning summary is asked for, and how detailed.
//
//demi:enum
//demi:export
type ThinkingSummary string

const (
	ThinkingSummaryAuto     ThinkingSummary = "auto"
	ThinkingSummaryConcise  ThinkingSummary = "concise"
	ThinkingSummaryDetailed ThinkingSummary = "detailed"
	ThinkingSummaryOff      ThinkingSummary = "off"
	ThinkingSummaryOn       ThinkingSummary = "on"
)

// One way a model can think, as its catalog offers it. Effort levels are the
// vendor's words, such as `low` or `xhigh`.
//
//demi:union tag=type
//demi:export
type ThinkingCapability interface{ isThinkingCapability() }

//demi:variant adaptive
type ThinkingCapabilityAdaptive struct {
	Efforts       []string `json:"efforts"`
	DefaultEffort *string  `json:"defaultEffort" check:"nullable"`
}

func (ThinkingCapabilityAdaptive) isThinkingCapability() {}

//demi:variant budget
type ThinkingCapabilityBudget struct {
	MinBudgetTokens     *uint32 `json:"minBudgetTokens" check:"nullable"`
	MaxBudgetTokens     *uint32 `json:"maxBudgetTokens" check:"nullable"`
	DefaultBudgetTokens *uint32 `json:"defaultBudgetTokens" check:"nullable"`
}

func (ThinkingCapabilityBudget) isThinkingCapability() {}

//demi:variant effort
type ThinkingCapabilityEffort struct {
	Efforts        []string          `json:"efforts"`
	DefaultEffort  *string           `json:"defaultEffort" check:"nullable"`
	Summaries      []ThinkingSummary `json:"summaries"`
	DefaultSummary *ThinkingSummary  `json:"defaultSummary" check:"nullable"`
}

func (ThinkingCapabilityEffort) isThinkingCapability() {}

// Thinking can be turned off.
//
//demi:variant disabled
type ThinkingCapabilityDisabled struct {
}

func (ThinkingCapabilityDisabled) isThinkingCapability() {}

// The thinking setting a selection makes, which each provider maps onto its
// vendor's option.
//
//demi:union tag=type
//demi:export
type ThinkingConfig interface{ isThinkingConfig() }

//demi:variant adaptive
type ThinkingConfigAdaptive struct {
	Effort string `json:"effort"`
}

func (ThinkingConfigAdaptive) isThinkingConfig() {}

//demi:variant budget
type ThinkingConfigBudget struct {
	BudgetTokens uint32 `json:"budgetTokens"`
}

func (ThinkingConfigBudget) isThinkingConfig() {}

//demi:variant effort
type ThinkingConfigEffort struct {
	Effort  string           `json:"effort"`
	Summary *ThinkingSummary `json:"summary" check:"nullable"`
}

func (ThinkingConfigEffort) isThinkingConfig() {}

//demi:variant disabled
type ThinkingConfigDisabled struct {
}

func (ThinkingConfigDisabled) isThinkingConfig() {}
