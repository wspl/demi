package provider

import (
	"fmt"
	"unicode/utf16"
)

// ShortHash is the vendor ID hash: FNV-1a over UTF-16 units, without zero padding.
func ShortHash(text string) string {
	hash := uint32(2166136261)
	for _, unit := range utf16.Encode([]rune(text)) {
		hash ^= uint32(unit)
		hash *= 16777619
	}
	return fmt.Sprintf("%x", hash)
}

// PromptCacheKey preserves session IDs up to 64 UTF-16 units.
func PromptCacheKey(sessionID string) string {
	if len(utf16.Encode([]rune(sessionID))) <= 64 {
		return sessionID
	}
	return "session_" + ShortHash(sessionID)
}
