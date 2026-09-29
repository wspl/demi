package core

import "slices"

const ThinkingOff = "disabled"

var AttachmentFileExtensions = []FileExtension{FileExtensionPng, FileExtensionJpg, FileExtensionJpeg, FileExtensionGif, FileExtensionWebp, FileExtensionPdf}
var VideoFileExtensions = []FileExtension{FileExtensionMp4, FileExtensionMov, FileExtensionWebm, FileExtensionM4v}

// FileExtensionSupport returns nil when the catalog does not state support.
func FileExtensionSupport(accepted *[]FileExtension, extension FileExtension) *bool {
	if accepted == nil {
		return nil
	}
	alias := extension
	if extension == FileExtensionJpg {
		alias = FileExtensionJpeg
	}
	if extension == FileExtensionJpeg {
		alias = FileExtensionJpg
	}
	supported := slices.Contains(*accepted, extension) || slices.Contains(*accepted, alias)
	return &supported
}

func ModelAcceptsVideo(model Model) bool {
	for _, extension := range VideoFileExtensions {
		supported := FileExtensionSupport(model.AcceptedExtensions, extension)
		if supported != nil && *supported {
			return true
		}
	}
	return false
}

func (s ModelSelection) ThinkingEffort() *string {
	if s.Thinking == nil {
		return nil
	}
	switch config := (*s.Thinking).(type) {
	case ThinkingConfigAdaptive:
		return &config.Effort
	case *ThinkingConfigAdaptive:
		if config != nil {
			return &config.Effort
		}
	case ThinkingConfigEffort:
		return &config.Effort
	case *ThinkingConfigEffort:
		if config != nil {
			return &config.Effort
		}
	case ThinkingConfigDisabled, *ThinkingConfigDisabled:
		effort := ThinkingOff
		return &effort
	}
	return nil
}
