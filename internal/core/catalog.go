package core

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
	return ModelSelection{
		ProviderID: providerID,
		Model: Model{
			ID:                 m.ID,
			Name:               m.DisplayName,
			ContextWindow:      window,
			OutputLimit:        m.OutputLimit,
			Thinking:           m.ThinkingCapabilities(),
			AcceptedExtensions: m.AcceptedFileExtensions(),
		},
		Thinking:      thinking,
		ServiceTierID: tier,
	}
}

// UnavailableSetting identifies an effort or tier the catalog does not offer.
type UnavailableSetting struct {
	Kind  string `json:"kind"`
	Value string `json:"value"`
}

func (e *UnavailableSetting) Error() string {
	return fmt.Sprintf("The model does not offer the %s %q", e.Kind, e.Value)
}

// ThinkingFor resolves an explicit effort, Off, or the model's default.
func (m ProviderModel) ThinkingFor(effort *string) (ThinkingConfig, error) {
	if effort == nil {
		effort = m.UnnamedEffort()
		if effort == nil {
			return nil, nil
		}
	}
	unavailable := &UnavailableSetting{Kind: "thinking effort", Value: *effort}
	efforts := m.thinkingEfforts()
	if len(efforts) == 0 {
		return nil, unavailable
	}
	if *effort == ThinkingOff {
		if m.CanDisableThinking != nil && !*m.CanDisableThinking {
			return nil, unavailable
		}
		return &DisabledConfig{}, nil
	}
	if !slices.Contains(efforts, *effort) {
		return nil, unavailable
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
		return m.DefaultThinkingEffort
	}
	if len(efforts) > 0 {
		return &efforts[0]
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
			return tier, nil
		}
	}
	return nil, &UnavailableSetting{Kind: "service tier", Value: *tier}
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
	return []ThinkingCapability{&EffortCapability{
		Efforts: slices.Clone(efforts), DefaultEffort: m.DefaultThinkingEffort,
		Summaries: []ThinkingSummary{ThinkingSummaryAuto, ThinkingSummaryConcise, ThinkingSummaryDetailed, ThinkingSummaryOff, ThinkingSummaryOn},
	}}
}

// ThinkingEffort gives the effort displayed by the conversation's settings.
func (m ModelSelection) ThinkingEffort() *string {
	switch c := m.Thinking.(type) {
	case *AdaptiveConfig:
		if c != nil {
			return &c.Effort
		}
	case *EffortConfig:
		if c != nil {
			return &c.Effort
		}
	case *DisabledConfig:
		off := ThinkingOff
		return &off
	case *BudgetConfig, nil:
		return nil
	}
	return nil
}
