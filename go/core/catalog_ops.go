package core

import (
	"fmt"
	"slices"
)

// A part of a conversation's model settings that its model does not offer.
type UnavailableSetting struct {
	Kind  string
	Value string
}

func (e *UnavailableSetting) Error() string {
	return fmt.Sprintf("The model does not offer the %s \"%s\"", e.Kind, e.Value)
}

func (m ProviderModel) Selection(providerID string, thinking *ThinkingConfig, serviceTierID *string) ModelSelection {
	contextWindow := uint32(0)
	if m.ContextWindow != nil {
		contextWindow = *m.ContextWindow
	}
	var outputLimit *uint32
	if m.OutputLimit != nil {
		outputLimit = new(*m.OutputLimit)
	}
	return ModelSelection{
		ProviderID: providerID,
		Model: Model{
			ID:                 m.ID,
			Name:               m.DisplayName,
			ContextWindow:      contextWindow,
			OutputLimit:        outputLimit,
			Thinking:           m.ThinkingCapabilities(),
			AcceptedExtensions: m.AcceptedFileExtensions(),
		},
		Thinking:      thinking,
		ServiceTierID: serviceTierID,
	}
}

func (m ProviderModel) ThinkingFor(effort *string) (*ThinkingConfig, error) {
	if effort == nil {
		effort = m.UnnamedEffort()
	}
	if effort == nil {
		return nil, nil
	}
	capabilities := m.ThinkingCapabilities()
	if *effort == ThinkingOff {
		leveled := false
		for _, capability := range capabilities {
			switch capability.(type) {
			case ThinkingCapabilityAdaptive, ThinkingCapabilityEffort:
				leveled = true
			}
		}
		if leveled && (m.CanDisableThinking == nil || *m.CanDisableThinking) {
			var config ThinkingConfig = ThinkingConfigDisabled{}
			return &config, nil
		}
	} else {
		for _, capability := range capabilities {
			switch c := capability.(type) {
			case ThinkingCapabilityAdaptive:
				if slices.Contains(c.Efforts, *effort) {
					var config ThinkingConfig = ThinkingConfigAdaptive{Effort: *effort}
					return &config, nil
				}
			case ThinkingCapabilityEffort:
				if slices.Contains(c.Efforts, *effort) {
					var config ThinkingConfig = ThinkingConfigEffort{Effort: *effort, Summary: c.DefaultSummary}
					return &config, nil
				}
			}
		}
	}
	return nil, &UnavailableSetting{Kind: "thinking effort", Value: *effort}
}

func (m ProviderModel) UnnamedEffort() *string {
	if m.CanDisableThinking == nil || *m.CanDisableThinking {
		return nil
	}
	for _, capability := range m.ThinkingCapabilities() {
		var efforts []string
		var defaultEffort *string
		switch c := capability.(type) {
		case ThinkingCapabilityAdaptive:
			efforts = c.Efforts
			defaultEffort = c.DefaultEffort
		case ThinkingCapabilityEffort:
			efforts = c.Efforts
			defaultEffort = c.DefaultEffort
		default:
			continue
		}
		if defaultEffort != nil && slices.Contains(efforts, *defaultEffort) {
			value := *defaultEffort
			return &value
		}
		if len(efforts) > 0 {
			return &efforts[0]
		}
	}
	return nil
}

func (m ProviderModel) TierFor(tier *string) (*string, error) {
	if tier == nil {
		return nil, nil
	}
	for _, listed := range m.ServiceTiers {
		if listed.ID == *tier {
			value := *tier
			return &value, nil
		}
	}
	return nil, &UnavailableSetting{Kind: "service tier", Value: *tier}
}

func (m ProviderModel) AcceptedFileExtensions() *[]FileExtension {
	if m.AcceptedExtensions != nil {
		extensions := slices.Clone(*m.AcceptedExtensions)
		return &extensions
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

func (m ProviderModel) ThinkingCapabilities() []ThinkingCapability {
	if m.SupportsReasoning != nil && !*m.SupportsReasoning {
		return []ThinkingCapability{ThinkingCapabilityDisabled{}}
	}
	if m.SupportedThinkingEfforts == nil || len(*m.SupportedThinkingEfforts) == 0 {
		return []ThinkingCapability{}
	}
	var defaultEffort *string
	if m.DefaultThinkingEffort != nil {
		defaultEffort = new(*m.DefaultThinkingEffort)
	}
	return []ThinkingCapability{ThinkingCapabilityEffort{
		Efforts:       slices.Clone(*m.SupportedThinkingEfforts),
		DefaultEffort: defaultEffort,
		Summaries:     []ThinkingSummary{ThinkingSummaryAuto, ThinkingSummaryConcise, ThinkingSummaryDetailed, ThinkingSummaryOff, ThinkingSummaryOn},
	}}
}
