package provider

import (
	"context"
	"io"
	"math"
	"net/http"
	"sort"
	"strconv"
	"strings"
	"unicode/utf8"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/core"
)

//go:generate go run github.com/wspl/demi/tools/contractgen

// HTTPFailureRecord is what an HTTP failure keeps as its upstream: the
// status, every response header as a [name, value] pair with the name in
// lowercase, sorted by name with a repeated header kept as separate pairs
// in arrival order, and the body text as received. Nothing in it is parsed,
// filtered or redacted.
// +demi:root
type HTTPFailureRecord struct {
	Status  uint16       `json:"status"`
	Headers []HeaderPair `json:"headers"`
	Body    string       `json:"body"`
}

// HeaderPair is one response header's name and value in arrival order.
// +demi:length min=2 max=2
type HeaderPair []string

// NewHTTPFailureRecord preserves a response with lowercase, stably sorted headers.
func NewHTTPFailureRecord(status uint16, headers http.Header, body string) HTTPFailureRecord {
	pairs := make([]HeaderPair, 0, len(headers))
	for name, values := range headers {
		for _, value := range values {
			pairs = append(pairs, HeaderPair{strings.ToLower(name), vendorResponseText([]byte(value))})
		}
	}
	sort.SliceStable(pairs, func(i, j int) bool { return pairs[i][0] < pairs[j][0] })
	return HTTPFailureRecord{Status: status, Headers: pairs, Body: body}
}

// Header returns the first value of a header; ok is false when it is absent.
func (r HTTPFailureRecord) Header(name string) (string, bool) {
	for _, pair := range r.Headers {
		if len(pair) == 2 && strings.EqualFold(pair[0], name) {
			return pair[1], true
		}
	}
	return "", false
}

// ReadHTTPRecord reads only HTTP diagnostics containing a valid failure record.
func ReadHTTPRecord(d *core.ProviderErrorDiagnostics) (HTTPFailureRecord, bool) {
	if d.Source != "http" || d.Upstream == nil {
		return HTTPFailureRecord{}, false
	}
	record, err := DecodeHTTPFailureRecord([]byte(*d.Upstream))
	if err != nil {
		return HTTPFailureRecord{}, false
	}
	return record, true
}

// HTTPFailure reads and closes a refused response; an unreadable body counts as empty.
func HTTPFailure(
	ctx context.Context,
	response *http.Response,
	label string,
	reader FailureReader,
	clock core.Clock,
) Failure {
	stop := closeOnCancel(ctx, response.Body)
	defer stop()
	body, err := io.ReadAll(response.Body)
	if err != nil {
		body = nil
	}
	return Refused(label, uint16(response.StatusCode), response.Header, vendorResponseText(body), reader, clock.Now())
}

// Refused records an HTTP failure that has already been read.
func Refused(
	label string,
	status uint16,
	headers http.Header,
	body string,
	reader FailureReader,
	receivedAt core.Timestamp,
) Failure {
	message := label + " API request failed with HTTP " + strconv.Itoa(int(status))
	if body != "" {
		message += ": " + body
	}
	record := NewHTTPFailureRecord(status, headers, body)
	encoded, err := contract.EncodeJSON(record)
	if err != nil {
		return Failure{Message: "could not encode HTTP failure record"}
	}
	upstream := string(encoded)
	return (Failure{
		Message: message,
		Code:    HTTPErrorCode(int(status), message),
		Diagnostics: &core.ProviderErrorDiagnostics{
			Source: "http", HTTPStatus: &status, Upstream: &upstream,
		},
	}).WithRetryWait(
		reader,
		receivedAt,
	)
}

// ReadHTTPFailure is the standard Retry-After reading of a failure record.
func ReadHTTPFailure(d *core.ProviderErrorDiagnostics, receivedAt core.Timestamp) core.ProviderFailureFacts {
	record, ok := ReadHTTPRecord(d)
	if !ok {
		return core.ProviderFailureFacts{}
	}
	header, ok := record.Header("retry-after")
	if !ok {
		return core.ProviderFailureFacts{}
	}
	return core.ProviderFailureFacts{RetryAt: RetryAt(header, receivedAt)}
}

// RetryAt reads decimal seconds after receipt or an RFC 9110 HTTP date.
func RetryAt(value string, receivedAt core.Timestamp) *core.Timestamp {
	value = strings.Trim(value, " \t\n\r\v\f")
	if decimalSeconds.MatchString(value) {
		whole, fraction, _ := strings.Cut(value, ".")
		seconds, err := strconv.ParseInt(whole, 10, 64)
		if err != nil || seconds > math.MaxInt64/1000 {
			return nil
		}
		fraction = (fraction + "000")[:3]
		part, _ := strconv.ParseInt(fraction, 10, 64) // Three ASCII digits by the pattern.
		if seconds*1000 > math.MaxInt64-part {
			return nil
		}
		delay := seconds*1000 + part
		start, err := receivedAt.Millisecond()
		if err != nil || start > math.MaxInt64-delay {
			return nil
		}
		at, err := core.TimestampFromMillisecond(start + delay)
		if err != nil {
			return nil
		}
		return &at
	}
	date, err := http.ParseTime(value)
	if err != nil || date.Unix() < 0 {
		return nil
	}
	at, err := core.TimestampFromTime(date)
	if err != nil {
		return nil
	}
	return &at
}

// vendorResponseText keeps a refused response's text, replacing each
// malformed UTF-8 sequence with one U+FFFD.
func vendorResponseText(data []byte) string {
	if utf8.Valid(data) {
		return string(data)
	}
	var text strings.Builder
	for len(data) > 0 {
		ch, width := utf8.DecodeRune(data)
		if ch == utf8.RuneError && width == 1 {
			width = vendorInvalidUTF8Width(data)
		}
		text.WriteRune(ch)
		data = data[width:]
	}
	return text.String()
}
