package provider

import (
	"errors"
	"fmt"
	"net"
	"net/url"
	"regexp"
	"strings"
	"time"

	"github.com/wspl/demi/internal/types"
)

// FailureReader reads the facts of a stored vendor failure at its receipt time.
type FailureReader func(*types.ProviderErrorDiagnostics, types.Timestamp) types.ProviderFailureFacts

// Failure is a failed run: the run's last event.
type Failure struct {
	Message     string
	Code        *ErrorCode
	Diagnostics *types.ProviderErrorDiagnostics
	RetryAfter  *time.Duration
}

// Error returns the diagnostic for this failure.
func (f *Failure) Error() string {
	return f.Message
}

// ErrorCode is a recovery code or an unrecognized vendor code, retained verbatim.
type ErrorCode string

// Recovery codes understood by the agent.
const (
	// RateLimit identifies a vendor rate limit.
	RateLimit ErrorCode = "rate_limit"
	// RateLimited identifies a vendor rate-limited response.
	RateLimited ErrorCode = "rate_limited"
	// Overloaded identifies an unavailable or overloaded vendor.
	Overloaded ErrorCode = "overloaded"
	// ContextLengthExceeded identifies a request beyond the model context or output limits.
	ContextLengthExceeded ErrorCode = "context_length_exceeded"
	// Incomplete identifies an incomplete vendor response.
	Incomplete ErrorCode = "incomplete"
	// AuthExpired identifies expired credentials.
	AuthExpired ErrorCode = "auth_expired"
	// AuthMissing identifies absent credentials.
	AuthMissing ErrorCode = "auth_missing"
	// AuthInvalid identifies invalid credentials.
	AuthInvalid ErrorCode = "auth_invalid"
	// AuthUnsupported identifies unsupported authentication.
	AuthUnsupported ErrorCode = "auth_unsupported"
	// AuthRefreshFailed identifies a failed credential refresh.
	AuthRefreshFailed ErrorCode = "auth_refresh_failed"
)

var (
	tooLarge   = regexp.MustCompile(`(?i)context|too long|token|too large|too many images|many-image`)
	categories = []struct {
		code    ErrorCode
		pattern *regexp.Regexp
	}{
		{
			ContextLengthExceeded,
			regexp.MustCompile(
				`\bcontext\b|\btoo long\b|\btoo large\b|\btoo many images\b|` +
					`\bmany image\b|\bmax\w*\b.*\btokens?\b`,
			),
		},
		{RateLimit, regexp.MustCompile(`\brate\b|\bratelimit\w*|\bquota\b|\busage limit\b|\bbilling\b|\bbalance\b`)},
		{
			AuthExpired,
			regexp.MustCompile(
				`\bauth(?:entication|orization)?\b|(?:invalid|expired).*(?:api|` +
					`access|auth) ?(?:key|token)|(?:api|access|auth) ?(?:key|` +
					`token).*(?:invalid|expired)`,
			),
		},
		{
			Overloaded,
			regexp.MustCompile(
				`\boverload|\bunavailable\b|\b(?:server|internal|api) error\b|` +
					`\btimed? ?out|\bfetch failed\b|\bnetwork\b|\bsocket\b|\beconn`,
			),
		},
	}
)
var nonWords = regexp.MustCompile(`[^a-z0-9]+`)

// HTTPErrorCode classifies HTTP failures by status and oversized-request text.
func HTTPErrorCode(status int, message string) *ErrorCode {
	var code ErrorCode
	switch {
	case status == 401 || status == 403:
		code = AuthExpired
	case status == 429:
		code = RateLimit
	case status == 408 || status == 409 || status == 425 || status >= 500:
		code = Overloaded
	case status == 413 || status == 400 && tooLarge.MatchString(message):
		code = ContextLengthExceeded
	default:
		return nil
	}
	return &code
}

// ClassifyError classifies vendor-reported words, falling back to the vendor code.
func ClassifyError(code *string, message string) *ErrorCode {
	text := message
	if code != nil {
		text = *code + " " + text
	}
	text = strings.TrimSpace(nonWords.ReplaceAllString(strings.ToLower(text), " "))
	for _, category := range categories {
		if category.pattern.MatchString(text) {
			value := category.code
			return &value
		}
	}
	if code != nil && *code != "" {
		value := ErrorCode(*code)
		return &value
	}
	return nil
}

// ProtocolFailure preserves an undecodable frame without a retryable code.
func ProtocolFailure(message, received string) Failure {
	return Failure{
		Message:     message,
		Diagnostics: &types.ProviderErrorDiagnostics{Source: "stream", Upstream: &received},
	}
}

// NoAnswer reports a transient failure without a vendor record.
func NoAnswer(message string) Failure {
	code := Overloaded
	return Failure{Message: message, Code: &code, Diagnostics: &types.ProviderErrorDiagnostics{Source: "transport"}}
}

// TransportFailure reports an HTTP transport failure without exposing its endpoint.
func TransportFailure(label string, err error) Failure {
	var request *url.Error
	if errors.As(err, &request) {
		err = request.Err
	}
	// net.OpError includes the destination address. Its cause preserves the
	// failure without exposing the endpoint (including credentials in URLs).
	var operation *net.OpError
	if errors.As(err, &operation) {
		err = operation.Err
	}
	return NoAnswer(fmt.Sprintf("%s API request failed: %v", label, err))
}

// RequestBuildFailure reports a request that could not be constructed, without
// a retryable code. Go reports construction separately from Client.Do.
func RequestBuildFailure(label string, err error) Failure {
	failure := TransportFailure(label, err)
	failure.Code = nil
	return failure
}

// WithRetryWait sets the wait named by the vendor's failure reader.
func (f Failure) WithRetryWait(reader FailureReader, now types.Timestamp) Failure {
	if f.Diagnostics == nil || reader == nil {
		return f
	}
	at := reader(f.Diagnostics, now).RetryAt
	if at == nil {
		return f
	}
	start, err := now.Time()
	if err != nil {
		return f
	}
	end, err := at.Time()
	if err != nil {
		return f
	}
	wait := max(time.Duration(0), end.Sub(start))
	f.RetryAfter = &wait
	return f
}

// EventStreamFailure maps broken bodies to transport failures and invalid text to protocol failures.
func EventStreamFailure(label string, err error) Failure {
	var stream *SSEError
	if !errors.As(err, &stream) {
		return TransportFailure(label, err)
	}
	if stream.Kind == SSETransport {
		return TransportFailure(label, stream.Err)
	}
	message := label + " API stream cannot be parsed: " + stream.Err.Error()
	if stream.Kind == SSEUTF8 {
		message = label + " API stream is not UTF-8 text: " + stream.Err.Error()
	}
	return Failure{Message: message, Diagnostics: &types.ProviderErrorDiagnostics{Source: "stream"}}
}
