package provider

import (
	"encoding/json/v2"
	"errors"
	"fmt"
	"io"
	"math"
	"net"
	"net/http"
	"net/url"
	"strconv"
	"strings"

	"github.com/wspl/demi/go/core"
)

type FailureReader func(core.ProviderErrorDiagnostics, core.Timestamp) core.ProviderFailureFacts
type ProviderFailure struct {
	Message     string
	Code        ErrorCode
	Diagnostics *core.ProviderErrorDiagnostics
	// RetryAfter is milliseconds, preserving the timestamp range without nanosecond overflow.
	RetryAfter *uint64
}

func (f ProviderFailure) Error() string { return f.Message }
func ProtocolFailure(message, received string) ProviderFailure {
	return ProviderFailure{Message: message, Diagnostics: &core.ProviderErrorDiagnostics{Source: core.FailureSourceStream, Upstream: &received}}
}
func NoAnswer(message string) ProviderFailure {
	return ProviderFailure{Message: message, Code: Overloaded, Diagnostics: &core.ProviderErrorDiagnostics{Source: core.FailureSourceTransport}}
}
func TransportFailure(label string, err error) ProviderFailure {
	return NoAnswer(fmt.Sprintf("%s API request failed: %v", label, WithoutEndpoint(err)))
}

// WithoutEndpoint returns err without the endpoint it names, which stays
// inside the provider: a request's URL, the address it dialed and the host it
// looked up give way to their causes.
func WithoutEndpoint(err error) error {
	var request *url.Error
	if errors.As(err, &request) {
		err = request.Err
	}
	var dial *net.OpError
	if errors.As(err, &dial) {
		err = dial.Err
	}
	var lookup *net.DNSError
	if errors.As(err, &lookup) {
		err = errors.New(lookup.Err)
	}
	return err
}
func EventStreamFailure(label string, err error) ProviderFailure {
	if errors.Is(err, ErrSSEUTF8) {
		return ProviderFailure{Message: label + " API stream is not UTF-8 text", Diagnostics: &core.ProviderErrorDiagnostics{Source: core.FailureSourceStream}}
	}
	return TransportFailure(label, err)
}
func (f ProviderFailure) WithRetryWait(reader FailureReader, now core.Timestamp) ProviderFailure {
	if f.Diagnostics == nil {
		return f
	}
	facts := reader(*f.Diagnostics, now)
	if facts.RetryAt == nil {
		return f
	}
	wait := uint64(max(0, facts.RetryAt.Millisecond()-now.Millisecond()))
	f.RetryAfter = &wait
	return f
}
func ReadHTTPFailureRecord(d core.ProviderErrorDiagnostics) *HTTPFailureRecord {
	if d.Source != core.FailureSourceHTTP || d.Upstream == nil {
		return nil
	}
	var record HTTPFailureRecord
	err := json.Unmarshal([]byte(*d.Upstream), &record)
	if err != nil {
		return nil
	}
	return &record
}
func Refused(label string, status uint16, headers http.Header, body string, reader FailureReader, received core.Timestamp) ProviderFailure {
	message := fmt.Sprintf("%s API request failed with HTTP %d", label, status)
	if body != "" {
		message += ": " + body
	}
	record := NewHTTPFailureRecord(status, headers, body)
	encoded, err := record.JSON()
	if err != nil {
		panic(err)
	} // A record contains only valid strings and integers.
	upstream := string(encoded)
	return (ProviderFailure{Message: message, Code: HTTPErrorCode(status, message), Diagnostics: &core.ProviderErrorDiagnostics{Source: core.FailureSourceHTTP, HTTPStatus: &status, Upstream: &upstream}}).WithRetryWait(reader, received)
}

// HTTPFailure consumes and closes a refused response; unreadable bodies are empty.
func HTTPFailure(response *http.Response, label string, reader FailureReader, clock core.Clock) ProviderFailure {
	body, err := io.ReadAll(response.Body)
	response.Body.Close() // Closing a consumed or broken body cannot change the refusal.
	if err != nil {
		body = nil
	}
	return Refused(label, uint16(response.StatusCode), response.Header, strings.ToValidUTF8(string(body), "\uFFFD"), reader, clock.Now())
}
func ReadHTTPFailure(d core.ProviderErrorDiagnostics, received core.Timestamp) core.ProviderFailureFacts {
	record := ReadHTTPFailureRecord(d)
	if record == nil {
		return core.ProviderFailureFacts{}
	}
	value, ok := record.Header("retry-after")
	if !ok {
		return core.ProviderFailureFacts{}
	}
	return core.ProviderFailureFacts{RetryAt: RetryAt(value, received)}
}
func RetryAt(value string, received core.Timestamp) *core.Timestamp {
	value = strings.Trim(value, " \t\r\n\v\f")
	whole, fraction, hasFraction := strings.Cut(value, ".")
	digits := func(s string) bool {
		if s == "" {
			return false
		}
		for _, c := range []byte(s) {
			if c < '0' || c > '9' {
				return false
			}
		}
		return true
	}
	if digits(whole) && (!hasFraction || digits(fraction)) {
		seconds, err := strconv.ParseInt(whole, 10, 64)
		if err != nil || seconds > math.MaxInt64/1000 {
			return nil
		}
		ms := seconds * 1000
		scale := int64(100)
		for i := 0; i < min(3, len(fraction)); i++ {
			part := int64(fraction[i]-'0') * scale
			if ms > math.MaxInt64-part {
				return nil
			}
			ms += part
			scale /= 10
		}
		now := received.Millisecond()
		if now > math.MaxInt64-ms {
			return nil
		}
		stamp, err := core.TimestampFromMillisecond(now + ms)
		if err != nil {
			return nil
		}
		return &stamp
	}
	date, err := http.ParseTime(value)
	if err != nil || date.Unix() < 0 {
		return nil
	}
	stamp, err := core.TimestampFromMillisecond(date.UnixMilli())
	if err != nil {
		return nil
	}
	return &stamp
}
