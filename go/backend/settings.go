package backend

//go:generate go run ./internal/zonenames

import (
	"errors"
	"fmt"
	"slices"
	"strings"

	"golang.org/x/text/language"

	"github.com/wspl/demi/go/commandservice"
)

// errNotBCP47 refuses a language tag that is not BCP 47.
var errNotBCP47 = errors.New("the tag is not BCP 47")

// CanonicalLocale is a reported locale as it is saved (web-api.md § User
// preferences): its time zone as the IANA database spells it, matched
// without regard to case, and each language once as its canonical BCP 47
// tag, in the order the browser reported them, as the browser's
// Intl.getCanonicalLocales gives it: zh-cn becomes zh-CN and iw becomes he.
// A time zone the backend does not know, or a tag that is not BCP 47, is
// refused naming its field.
func CanonicalLocale(locale commandservice.CommandLocale) (commandservice.CommandLocale, error) {
	index := slices.IndexFunc(zoneNames, func(zone string) bool { return strings.EqualFold(zone, locale.TimeZone) })
	if index < 0 {
		return commandservice.CommandLocale{}, fmt.Errorf("locale.timeZone: %q is not a time zone the backend knows", locale.TimeZone)
	}
	languages := make([]string, 0, len(locale.Languages))
	for index, tag := range locale.Languages {
		canonical, err := canonicalTag(tag)
		if err != nil {
			return commandservice.CommandLocale{}, fmt.Errorf("locale.languages[%d]: %q is not a BCP 47 language tag", index, tag)
		}
		if !slices.Contains(languages, canonical) {
			languages = append(languages, canonical)
		}
	}
	return commandservice.CommandLocale{TimeZone: zoneNames[index], Languages: languages}, nil
}

// canonicalTag is tag in its canonical BCP 47 form. x/text also reads "_"
// as a separator, which BCP 47 does not have, so such a tag is refused.
func canonicalTag(tag string) (string, error) {
	if strings.Contains(tag, "_") {
		return "", errNotBCP47
	}
	parsed, err := language.BCP47.Parse(tag)
	if err != nil {
		return "", err
	}
	return parsed.String(), nil
}
