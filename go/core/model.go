package core

//demi:wire
type Model struct {
	ID                 string               `json:"id" check:"chars=1.."`
	Name               string               `json:"name"`
	ContextWindow      uint32               `json:"contextWindow"`
	InputLimit         *uint32              `json:"inputLimit" check:"nullable"`
	OutputLimit        *uint32              `json:"outputLimit" check:"nullable,range=1.."`
	Thinking           []ThinkingCapability `json:"thinking"`
	AcceptedExtensions *[]FileExtension     `json:"acceptedExtensions" check:"nullable"`
}

//demi:wire
type ModelSelection struct {
	ProviderID    string          `json:"providerId" check:"chars=1.."`
	Model         Model           `json:"model"`
	Thinking      *ThinkingConfig `json:"thinking" check:"nullable"`
	ServiceTierID *string         `json:"serviceTierId" check:"nullable,chars=1.."`
}

//demi:wire
type TokenUsage struct {
	InputTokens      uint64 `json:"inputTokens" check:"range=..MaxSafeInteger"`
	OutputTokens     uint64 `json:"outputTokens" check:"range=..MaxSafeInteger"`
	CacheReadTokens  uint64 `json:"cacheReadTokens" check:"range=..MaxSafeInteger"`
	CacheWriteTokens uint64 `json:"cacheWriteTokens" check:"range=..MaxSafeInteger"`
}

//demi:enum
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

//demi:enum
type ThinkingSummary string

const (
	ThinkingSummaryAuto     ThinkingSummary = "auto"
	ThinkingSummaryConcise  ThinkingSummary = "concise"
	ThinkingSummaryDetailed ThinkingSummary = "detailed"
	ThinkingSummaryOff      ThinkingSummary = "off"
	ThinkingSummaryOn       ThinkingSummary = "on"
)

//demi:union tag=type
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

//demi:variant disabled
type ThinkingCapabilityDisabled struct {
}

func (ThinkingCapabilityDisabled) isThinkingCapability() {}

//demi:union tag=type
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
