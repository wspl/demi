package accounts

import (
	_ "embed"
	"errors"
	"fmt"
	"regexp"
	"slices"
	"strings"

	"github.com/wspl/demi/internal/commandproto"
	"github.com/wspl/demi/internal/webapiproto"
	"golang.org/x/text/language"
)

// CheckedPatch has a known IANA time zone and unique canonical language tags.
type CheckedPatch struct{ patch webapiproto.PreferencesPatch }

// Check validates a preference patch and canonicalizes its reported locale.
func Check(patch webapiproto.PreferencesPatch) (CheckedPatch, error) {
	if err := patch.Validate(); err != nil {
		return CheckedPatch{}, err
	}
	if patch.Locale != nil {
		locale, err := canonicalLocale(*patch.Locale)
		if err != nil {
			return CheckedPatch{}, err
		}
		patch.Locale = &locale
	}
	return CheckedPatch{patch: patch}, nil
}

// ICU's normalized names include aliases and are independent of the host's
// tzdata. The external oracle is documented in testdata/README.md.
//
//go:embed iana_names.txt
var ianaNames string

var normalizedZones = func() map[string]string {
	zones := make(map[string]string)
	for name := range strings.FieldsSeq(ianaNames) {
		zones[strings.ToLower(name)] = name
	}
	return zones
}()

func canonicalLocale(locale commandproto.CommandLocale) (commandproto.CommandLocale, error) {
	zone, ok := normalizedZones[strings.ToLower(locale.TimeZone)]
	if !ok || strings.ContainsFunc(locale.TimeZone, func(r rune) bool { return r > 127 }) {
		return commandproto.CommandLocale{}, fmt.Errorf(
			"locale.timeZone: %q is not a time zone the backend knows",
			locale.TimeZone,
		)
	}
	languages := make([]commandproto.LanguageTag, 0, len(locale.Languages))
	for i, tag := range locale.Languages {
		canonical, err := canonicalLanguage(string(tag))
		if err != nil {
			return commandproto.CommandLocale{}, fmt.Errorf(
				"locale.languages[%d]: %q is not a BCP 47 language tag",
				i,
				string(tag),
			)
		}
		if !slices.Contains(languages, commandproto.LanguageTag(canonical)) {
			languages = append(languages, commandproto.LanguageTag(canonical))
		}
	}
	return commandproto.CommandLocale{TimeZone: zone, Languages: languages}, nil
}

// x/text accepts underscore separators, grandfathered tags and private-use-only
// tags. localeSyntax refuses them: a tag starts with a 2- or 3-letter language
// and separates its subtags with hyphens.
var (
	localeSyntax = regexp.MustCompile(
		`(?i)^[a-z]{2,3}(-[a-z]{4})?(-([a-z]{2}|[0-9]{3}))?` +
			`(-([a-z0-9]{5,8}|[0-9][a-z0-9]{3}))*` +
			`((-[a-wy-z0-9](-[a-z0-9]{2,8})+)*)(-x(-[a-z0-9]{1,8})+)?$`,
	)
	errLanguage = errors.New("invalid locale")
)

func canonicalLanguage(tag string) (string, error) {
	if strings.ContainsFunc(tag, func(r rune) bool { return r > 127 }) || !localeSyntax.MatchString(tag) {
		return "", errLanguage
	}
	input, unknownVariants, err := languageSubtags(tag)
	if err != nil {
		return "", err
	}
	input, unknownVariants, canonicalizer := canonicalLanguageAliases(input, unknownVariants)
	parsed, err := canonicalizer.Parse(strings.Join(input, "-"))
	unknownBase := ""
	if err != nil {
		// ICU accepts unregistered primary languages. x/text cannot represent
		// them: parse the rest with und and preserve the original primary tag.
		var unknown language.ValueError
		if errors.As(err, &unknown) && unknown.Subtag() == input[0] {
			unknownBase = input[0]
			input[0] = "und"
			parsed, err = canonicalizer.Parse(strings.Join(input, "-"))
		}
		if err != nil {
			return "", err
		}
	}
	canonical := parsed.String()
	if unknownBase != "" {
		canonical = unknownBase + strings.TrimPrefix(canonical, "und")
	}
	if len(unknownVariants) != 0 {
		canonical = restoreLanguageVariants(canonical, unknownVariants)
	}
	return canonical, nil
}

// icuUnicodeKeywords retains the first Unicode keyword and omits its true
// value, as ICU's Locale parser does. x/text rejects repeated keys and retains
// true, so these two ICU rules must be applied before its extension parser.
func icuUnicodeKeywords(parts []string) []string {
	start := -1
	for i, part := range parts[1:] {
		if part == "x" {
			return parts
		}
		if part == "u" {
			start = i + 1
			break
		}
	}
	if start < 0 {
		return parts
	}
	end := start + 1
	for end < len(parts) && len(parts[end]) != 1 {
		end++
	}
	result := slices.Clone(parts[:start+1])
	seen := make(map[string]bool)
	for i := start + 1; i < end; {
		if len(parts[i]) != 2 {
			result = append(result, parts[i])
			i++
			continue
		}
		next := i + 1
		for next < end && len(parts[next]) != 2 {
			next++
		}
		if !seen[parts[i]] {
			seen[parts[i]] = true
			result = append(result, parts[i])
			if next != i+2 || parts[i+1] != "true" {
				result = append(result, parts[i+1:next]...)
			}
		}
		i = next
	}
	return append(result, parts[end:]...)
}

// languageSubtags separates ICU variants that x/text cannot represent from the parseable tag.
func languageSubtags(tag string) ([]string, []string, error) {
	parts := strings.Split(strings.ToLower(tag), "-")
	seen := make(map[string]bool)
	unknownVariants := []string{}
	input := []string{parts[0]}
	extension := false
	for i, part := range parts[1:] {
		if len(part) == 1 {
			if seen[part] {
				return nil, nil, errLanguage
			}
			seen[part] = true
			extension = true
		}
		if !extension && (len(part) >= 5 || (len(part) == 4 && part[0] >= '0' && part[0] <= '9')) {
			if seen[part] {
				return nil, nil, errLanguage
			}
			seen[part] = true
			if _, err := language.ParseVariant(part); err != nil {
				// ICU retains syntactically valid unregistered variants, which x/text
				// cannot represent. Keep them outside its tag and restore them below.
				unknownVariants = append(unknownVariants, part)
				continue
			}
		}
		input = append(input, part)
		if extension && part == "x" {
			// Private-use subtags are not extension singleton keys.
			input = append(input, parts[i+2:]...)
			break
		}
	}
	return input, unknownVariants, nil
}

// canonicalLanguageAliases applies ICU aliases before x/text canonicalization.
func canonicalLanguageAliases(input, unknownVariants []string) ([]string, []string, language.CanonType) {
	input = icuUnicodeKeywords(input)
	// x/text maps mo to ro-MD, whereas ICU canonicalizes only the language.
	if input[0] == "mo" {
		input[0] = "ro"
	}
	canonicalizer := language.Default | language.Macro
	// ICU retains nb; x/text's Macro mapping predates that CLDR decision.
	if input[0] == "nb" {
		canonicalizer = language.Default
	}
	// ICU's language/variant alias is not in x/text's canonicalizer.
	if input[0] == "zh" && slices.Contains(unknownVariants, "hakka") {
		input[0] = "hak"
		unknownVariants = slices.DeleteFunc(unknownVariants, func(v string) bool { return v == "hakka" })
	}
	return input, unknownVariants, canonicalizer
}

// restoreLanguageVariants inserts unregistered variants before extensions in sorted order.
func restoreLanguageVariants(canonical string, unknownVariants []string) string {
	parts := strings.Split(canonical, "-")
	end := len(parts)
	start := end
	for i, part := range parts[1:] {
		index := i + 1
		if len(part) == 1 {
			end = index
			break
		}
		if len(part) >= 5 || (len(part) == 4 && part[0] >= '0' && part[0] <= '9') {
			if start == len(parts) {
				start = index
			}
		}
	}
	if start > end {
		start = end
	}
	variants := append(unknownVariants, parts[start:end]...)
	slices.Sort(variants)
	canonical = strings.Join(parts[:start], "-") + "-" + strings.Join(variants, "-")
	if end < len(parts) {
		canonical += "-" + strings.Join(parts[end:], "-")
	}
	return canonical
}
