package skills

import (
	"fmt"
	"slices"
	"strings"
	"unicode"
	"unicode/utf8"
)

const (
	catalogMaxChars = 8000
	noneAvailable   = "No skills are available now."
	catalogHeader   = `The following skills provide specialized instructions for specific tasks.
When a task matches a skill's description, read its SKILL.md at the listed
location before you start, and resolve relative paths in it against the
skill's directory.

<available_skills>
`
)

// catalogEntry names a skill at the path the model can read on its Host.
type catalogEntry struct{ name, description, location string }

// renderCatalog shares the character budget equally among descriptions.
func renderCatalog(entries []catalogEntry) string {
	entries = slices.Clone(entries)
	slices.SortStableFunc(entries, func(a, b catalogEntry) int { return strings.Compare(a.name, b.name) })
	longest := 0
	for _, entry := range entries {
		longest = max(longest, utf8.RuneCountInString(entry.description))
	}
	full := catalogBlock(entries, longest, 0)
	if utf8.RuneCountInString(full) <= catalogMaxChars {
		return full
	}
	fits := func(length int) bool {
		return utf8.RuneCountInString(catalogBlock(entries, length, 0)) <= catalogMaxChars
	}
	if fits(1) {
		low, high := 1, longest
		for low < high {
			middle := (low + high + 1) / 2
			if fits(middle) {
				low = middle
			} else {
				high = middle - 1
			}
		}
		return catalogBlock(entries, low, 0)
	}
	for leftOut := 0; ; leftOut++ {
		shown := entries[:len(entries)-leftOut]
		text := catalogBlock(shown, -1, leftOut)
		if utf8.RuneCountInString(text) <= catalogMaxChars || len(shown) == 0 {
			return text
		}
	}
}

// catalogBlock writes the Agent Skills catalog's exact XML spelling.
func catalogBlock(entries []catalogEntry, length, leftOut int) string {
	var text strings.Builder
	text.WriteString(catalogHeader)
	for _, entry := range entries {
		text.WriteString("  <skill>\n    <name>")
		text.WriteString(catalogEscape.Replace(entry.name))
		text.WriteString("</name>\n")
		if length >= 0 {
			text.WriteString("    <description>")
			text.WriteString(catalogEscape.Replace(shortened(entry.description, length)))
			text.WriteString("</description>\n")
		}
		text.WriteString("    <location>")
		text.WriteString(catalogEscape.Replace(entry.location))
		text.WriteString("</location>\n  </skill>\n")
	}
	text.WriteString("</available_skills>")
	if leftOut > 0 {
		fmt.Fprintf(&text, "\n%d more skills are not listed.", leftOut)
	}
	return text.String()
}

// shortened cuts a catalog description at a word within its character budget.
func shortened(text string, length int) string {
	if utf8.RuneCountInString(text) <= length {
		return text
	}
	kept := string([]rune(text)[:max(0, length-1)])
	if space := strings.LastIndexFunc(kept, unicode.IsSpace); space > 0 {
		kept = kept[:space]
	}
	return strings.TrimRightFunc(kept, unicode.IsSpace) + "…"
}

var catalogEscape = strings.NewReplacer("&", "&amp;", "<", "&lt;", ">", "&gt;", "\"", "&quot;", "'", "&apos;")

// nextCatalog compares with the latest block actually visible to the model.
func nextCatalog(entries []catalogEntry, seen []string) *string {
	if len(entries) == 0 && len(seen) == 0 {
		return nil
	}
	text := noneAvailable
	if len(entries) > 0 {
		text = renderCatalog(entries)
	}
	if len(seen) > 0 && seen[len(seen)-1] == text {
		return nil
	}
	return &text
}
