package skills

import (
	"fmt"
	"strings"
	"unicode"
	"unicode/utf8"

	"github.com/goccy/go-yaml"
)

const skillMDMaxBytes = 256 * 1024

type parsedSkill struct {
	name                   string
	description            string
	disableModelInvocation bool
	warnings               []string
}

type frontMatter struct {
	Name                   *string `yaml:"name"`
	Description            *string `yaml:"description"`
	DisableModelInvocation *bool   `yaml:"disable-model-invocation"`
}

// parseSkill reads the three Agent Skills fields, retaining lenient warnings.
func parseSkill(directory string, text []byte) (parsedSkill, error) {
	if len(text) > skillMDMaxBytes {
		return parsedSkill{}, fmt.Errorf("SKILL.md is larger than 256 KiB")
	}
	if !utf8.Valid(text) {
		return parsedSkill{}, fmt.Errorf("SKILL.md is not UTF-8")
	}
	front, ok := skillFrontMatter(string(text))
	if !ok {
		return parsedSkill{}, fmt.Errorf("SKILL.md has no front matter")
	}
	var fields frontMatter
	// Strict decoding keeps duplicate/type checks. The format explicitly ignores
	// other fields, as Rust's FrontMatter does without deny_unknown_fields.
	if err := yaml.UnmarshalWithOptions([]byte(front), &fields, yaml.Strict(), yaml.AllowFieldPrefixes(""), yaml.CustomUnmarshaler[string](decodeSkillText), yaml.CustomUnmarshaler[bool](decodeSkillBool)); err != nil {
		return parsedSkill{}, fmt.Errorf("the front matter does not parse: %w", err)
	}
	if fields.Description == nil || strings.TrimSpace(*fields.Description) == "" {
		return parsedSkill{}, fmt.Errorf("the front matter has no description")
	}
	parsed := parsedSkill{name: directory, description: *fields.Description, warnings: []string{}}
	if fields.Name != nil {
		parsed.name = *fields.Name
		if !validName(parsed.name) {
			parsed.warnings = append(parsed.warnings, fmt.Sprintf("the name \"%s\" is not 1 to 64 lowercase letters, digits and single hyphens", parsed.name))
		}
		if parsed.name != directory {
			parsed.warnings = append(parsed.warnings, fmt.Sprintf("the name \"%s\" differs from its directory \"%s\"", parsed.name, directory))
		}
	}
	if utf8.RuneCountInString(parsed.description) > 1024 {
		parsed.warnings = append(parsed.warnings, "the description is longer than 1,024 characters")
	}
	if fields.DisableModelInvocation != nil {
		parsed.disableModelInvocation = *fields.DisableModelInvocation
	}
	return parsed, nil
}

// skillFrontMatter finds the opening and closing delimiter without parsing body Markdown.
func skillFrontMatter(text string) (string, bool) {
	text = strings.TrimPrefix(text, "\ufeff")
	first, rest, found := strings.Cut(text, "\n")
	if !found || strings.TrimRightFunc(first, unicode.IsSpace) != "---" {
		return "", false
	}
	start := len(first) + 1
	offset := start
	for line := range strings.SplitAfterSeq(rest, "\n") {
		if strings.TrimRightFunc(line, unicode.IsSpace) == "---" {
			return text[start:offset], true
		}
		offset += len(line)
	}
	return "", false
}
