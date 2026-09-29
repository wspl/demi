package provider

import (
	"encoding/json/jsontext"
	"encoding/json/v2"
	"net/http"
	"sort"
	"strings"

	"github.com/wspl/demi/go/internal/wire"
)

// HTTPFailureRecord preserves a vendor's answer, without interpreting its body.
//
//demi:wire
type HTTPFailureRecord struct {
	Status  uint16       `json:"status"`
	Headers []HeaderPair `json:"headers"`
	Body    string       `json:"body"`
}

// HeaderPair preserves one response header as the stored [name,value] tuple.
//
//demi:opaque
type HeaderPair struct {
	Name  string
	Value string
}

func (p *HeaderPair) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	if err := wire.BeginArray(dec); err != nil {
		return err
	}
	var pair [2]string
	for i := range pair {
		text, err := wire.ReadString(dec)
		if err != nil {
			return wire.In(wire.Index(i), err)
		}
		pair[i] = text
	}
	token, err := dec.ReadToken()
	if err != nil {
		return err
	}
	if token.Kind() != ']' {
		return &wire.InvalidError{Rule: "must have two elements"}
	}
	*p = HeaderPair{Name: pair[0], Value: pair[1]}
	return nil
}
func (p HeaderPair) MarshalJSONTo(enc *jsontext.Encoder) error {
	return json.MarshalEncode(enc, [2]string{p.Name, p.Value})
}
func NewHTTPFailureRecord(status uint16, headers http.Header, body string) HTTPFailureRecord {
	pairs := make([]HeaderPair, 0, len(headers))
	for name, values := range headers {
		for _, value := range values {
			pairs = append(pairs, HeaderPair{strings.ToLower(name), strings.ToValidUTF8(value, "\ufffd")})
		}
	}
	sort.SliceStable(pairs, func(i, j int) bool { return pairs[i].Name < pairs[j].Name })
	return HTTPFailureRecord{Status: status, Headers: pairs, Body: body}
}
func (r HTTPFailureRecord) Header(name string) (string, bool) {
	for _, pair := range r.Headers {
		if strings.EqualFold(pair.Name, name) {
			return pair.Value, true
		}
	}
	return "", false
}
func (r HTTPFailureRecord) JSON() ([]byte, error) { return json.Marshal(r, json.Deterministic(true)) }
