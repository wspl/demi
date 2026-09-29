package provider

import (
	"regexp"
	"strings"
)

// ErrorCode is a recovery category or an unchanged vendor code. Empty means
// that no code was reported; protocol failures are never classified by text.
type ErrorCode string

const (
	RateLimit             ErrorCode = "rate_limit"
	RateLimited           ErrorCode = "rate_limited"
	Overloaded            ErrorCode = "overloaded"
	ContextLengthExceeded ErrorCode = "context_length_exceeded"
	Incomplete            ErrorCode = "incomplete"
	AuthExpired           ErrorCode = "auth_expired"
	AuthMissing           ErrorCode = "auth_missing"
	AuthInvalid           ErrorCode = "auth_invalid"
	AuthUnsupported       ErrorCode = "auth_unsupported"
	AuthRefreshFailed     ErrorCode = "auth_refresh_failed"
)

var tooLarge = regexp.MustCompile(`(?i)context|too long|token|too large|too many images|many-image`)
var categories = []struct {
	code    ErrorCode
	pattern *regexp.Regexp
}{
	{ContextLengthExceeded, regexp.MustCompile(`\bcontext\b|\btoo long\b|\btoo large\b|\btoo many images\b|\bmany image\b|\bmax\w*\b.*\btokens?\b`)},
	{RateLimit, regexp.MustCompile(`\brate\b|\bratelimit\w*|\bquota\b|\busage limit\b|\bbilling\b|\bbalance\b`)},
	{AuthExpired, regexp.MustCompile(`\bauth(?:entication|orization)?\b|(?:invalid|expired).*(?:api|access|auth) ?(?:key|token)|(?:api|access|auth) ?(?:key|token).*(?:invalid|expired)`)},
	{Overloaded, regexp.MustCompile(`\boverload|\bunavailable\b|\b(?:server|internal|api) error\b|\btimed? ?out|\bfetch failed\b|\bnetwork\b|\bsocket\b|\beconn`)},
}

func HTTPErrorCode(status uint16, message string) ErrorCode {
	switch {
	case status == 401 || status == 403:
		return AuthExpired
	case status == 429:
		return RateLimit
	case status == 408 || status == 409 || status == 425 || status >= 500:
		return Overloaded
	case status == 413 || status == 400 && tooLarge.MatchString(message):
		return ContextLengthExceeded
	default:
		return ""
	}
}

// ClassifyVendorFailure reads only vendor-reported words, with the Rust's
// category precedence. Punctuation separates ASCII words.
func ClassifyVendorFailure(code, message string) ErrorCode {
	fields := strings.FieldsFunc(code+" "+message, func(r rune) bool { return !(r >= 'a' && r <= 'z' || r >= 'A' && r <= 'Z' || r >= '0' && r <= '9') })
	text := strings.ToLower(strings.Join(fields, " "))
	for _, category := range categories {
		if category.pattern.MatchString(text) {
			return category.code
		}
	}
	return ErrorCode(code)
}
