package types

import (
	"fmt"
	"slices"
)

// Selection records a catalog model's facts and the conversation's choices.
func (m ProviderModel) Selection(providerID string, thinking ThinkingConfig, tier *string) ModelSelection {
	var window uint32
	if m.ContextWindow != nil {
		window = *m.ContextWindow
	}
	selection := ModelSelection{
		ProviderID: providerID,
		Model: Model{
			ID:                 m.ID,
			Name:               m.DisplayName,
			ContextWindow:      window,
			Thinking:           m.ThinkingCapabilities(),
			AcceptedExtensions: m.AcceptedFileExtensions(),
		},
	}
	if m.OutputLimit != nil {
		selection.Model.OutputLimit = new(*m.OutputLimit)
	}
	if tier != nil {
		selection.ServiceTierID = new(*tier)
	}
	switch config := thinking.(type) {
	case *AdaptiveConfig:
		selection.Thinking = new(*config)
	case *BudgetConfig:
		selection.Thinking = new(*config)
	case *EffortConfig:
		setting := *config
		if config.Summary != nil {
			setting.Summary = new(*config.Summary)
		}
		selection.Thinking = &setting
	case *DisabledConfig:
		selection.Thinking = new(*config)
	case nil:
	}
	return selection
}

// settingUnavailable refuses a thinking effort or service tier the model does not offer.
func settingUnavailable(setting, value string) error {
	//nolint:staticcheck // ST1005: the page shows this text as the refusal's message.
	return fmt.Errorf("The model does not offer the %s %q", setting, value)
}

// ThinkingFor resolves an explicit effort, Off, or the model's default.
func (m ProviderModel) ThinkingFor(effort *string) (ThinkingConfig, error) {
	if effort == nil {
		effort = m.UnnamedEffort()
		if effort == nil {
			return nil, nil
		}
	}
	efforts := m.thinkingEfforts()
	if len(efforts) == 0 {
		return nil, settingUnavailable("thinking effort", *effort)
	}
	if *effort == ThinkingOff {
		if m.CanDisableThinking != nil && !*m.CanDisableThinking {
			return nil, settingUnavailable("thinking effort", *effort)
		}
		return &DisabledConfig{}, nil
	}
	if !slices.Contains(efforts, *effort) {
		return nil, settingUnavailable("thinking effort", *effort)
	}
	return &EffortConfig{Effort: *effort}, nil
}

// UnnamedEffort picks a default only for a model that cannot turn thinking off.
func (m ProviderModel) UnnamedEffort() *string {
	if m.CanDisableThinking == nil || *m.CanDisableThinking {
		return nil
	}
	efforts := m.thinkingEfforts()
	if m.DefaultThinkingEffort != nil && slices.Contains(efforts, *m.DefaultThinkingEffort) {
		return new(*m.DefaultThinkingEffort)
	}
	if len(efforts) > 0 {
		return new(efforts[0])
	}
	return nil
}

// thinkingEfforts reads only reasoning efforts the catalog says the model can use.
func (m ProviderModel) thinkingEfforts() []string {
	if m.SupportsReasoning != nil && !*m.SupportsReasoning {
		return nil
	}
	if m.SupportedThinkingEfforts == nil {
		return nil
	}
	return *m.SupportedThinkingEfforts
}

// TierFor checks a named service tier; nil means the vendor's default.
func (m ProviderModel) TierFor(tier *string) (*string, error) {
	if tier == nil {
		return nil, nil
	}
	for _, listed := range m.ServiceTiers {
		if listed.ID == *tier {
			return new(*tier), nil
		}
	}
	return nil, settingUnavailable("service tier", *tier)
}

// AcceptedFileExtensions preserves an exact list, otherwise derives known support.
func (m ProviderModel) AcceptedFileExtensions() *[]FileExtension {
	if m.AcceptedExtensions != nil {
		exact := slices.Clone(*m.AcceptedExtensions)
		return &exact
	}
	video := m.SupportsVideo != nil && *m.SupportsVideo
	if m.SupportsAttachments == nil && !video {
		return nil
	}
	extensions := []FileExtension{}
	if m.SupportsAttachments != nil && *m.SupportsAttachments {
		extensions = append(extensions, AttachmentFileExtensions...)
	}
	if video {
		extensions = append(extensions, VideoFileExtensions...)
	}
	return &extensions
}

// ThinkingCapabilities gives the portable choices from the catalog's reasoning facts.
func (m ProviderModel) ThinkingCapabilities() []ThinkingCapability {
	if m.SupportsReasoning != nil && !*m.SupportsReasoning {
		return []ThinkingCapability{&DisabledCapability{}}
	}
	efforts := m.thinkingEfforts()
	if len(efforts) == 0 {
		return []ThinkingCapability{}
	}
	var defaultEffort *string
	if m.DefaultThinkingEffort != nil {
		defaultEffort = new(*m.DefaultThinkingEffort)
	}
	return []ThinkingCapability{&EffortCapability{
		Efforts:       slices.Clone(efforts),
		DefaultEffort: defaultEffort,
		Summaries: []ThinkingSummary{
			ThinkingSummaryAuto,
			ThinkingSummaryConcise,
			ThinkingSummaryDetailed,
			ThinkingSummaryOff,
			ThinkingSummaryOn,
		},
	}}
}

// ThinkingEffort gives the effort displayed by the conversation's settings.
func (m ModelSelection) ThinkingEffort() (string, bool) {
	switch c := m.Thinking.(type) {
	case *AdaptiveConfig:
		return c.Effort, true
	case *EffortConfig:
		return c.Effort, true
	case *DisabledConfig:
		return ThinkingOff, true
	case *BudgetConfig, nil:
		return "", false
	}
	return "", false
}
