package provider_test

import (
	"errors"
	"fmt"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/provider"
)

type testText struct {
	Text string `json:"text"`
}
type testBlock struct{ Text *testText }

func (b *testBlock) UnmarshalJSON(data []byte) error {
	value, ok, err := provider.DecodeTagged(
		string(data),
		map[string]func(string) (testText, error){"text": provider.DecodeUntagged[testText]},
	)
	b.Text = nil
	if ok {
		b.Text = &value
	}
	return err
}

type testStart struct {
	Index uint32    `json:"index"`
	Block testBlock `json:"block"`
}

var testPayloads = map[string]func(string) (any, error){
	"start": func(text string) (any, error) {
		return provider.DecodeUntagged[testStart](text)
	},
	"stop": func(text string) (any, error) {
		return provider.DecodeUntagged[struct{}](text)
	},
}

func TestWireUnknownAndRegisteredTags(t *testing.T) {
	value, ok, err := provider.DecodeTagged(`{"type":"ping"}`, testPayloads)
	if err != nil || ok {
		t.Fatalf("%v %v", value, err)
	}
	value, _, err = provider.DecodeTagged(`{"type":"stop","vendor_field":1}`, testPayloads)
	if err != nil {
		t.Fatal(err)
	}
	requireEqual(t, value, struct{}{})
	value, _, err = provider.DecodeTagged(
		`{"type":"start","index":2,"block":{"type":"text","text":"hi"}}`,
		testPayloads,
	)
	if err != nil {
		t.Fatal(err)
	}
	requireEqual(t, value, testStart{Index: 2, Block: testBlock{Text: &testText{Text: "hi"}}})
	value, _, err = provider.DecodeTagged(`{"type":"start","index":0,"block":{"type":"image"}}`, testPayloads)
	if err != nil {
		t.Fatal(err)
	}
	requireEqual(t, value, testStart{})
}

func TestWireMalformedRegisteredPayload(t *testing.T) {
	_, _, err := provider.DecodeTagged(`{"type":"start","block":{"type":"text","text":"hi"}}`, testPayloads)
	if err == nil || !strings.Contains(err.Error(), "index") {
		t.Fatalf("%v", err)
	}
	_, _, err = provider.DecodeTagged(`{"type":"start","index":0,"block":{"type":"text","text":42}}`, testPayloads)
	var wire *provider.WireError
	// Only the field prefix is fixed; the decoder's wording after it may vary.
	if !errors.As(err, &wire) || wire.Path() != "block" || !strings.HasPrefix(err.Error(), "block: text: ") {
		t.Fatalf("%v", err)
	}
}

func TestWireMissingTagOrInvalidJSON(t *testing.T) {
	for _, text := range []string{`{}`, `{"type":5}`, "not json", "", `{"type":"start","index":0,"block":{"text":"hi"}}`} {
		if _, _, err := provider.DecodeTagged(text, testPayloads); err == nil {
			t.Fatalf("accepted %s", text)
		}
	}
}

func TestWireReportedFieldsAndWholeTokens(t *testing.T) {
	type report struct {
		Message provider.ReportedString `json:"message" wire:"optional"`
		Tokens  *uint64                 `json:"tokens"`
	}
	value, err := provider.DecodeUntagged[report](`{"message":{"nested":true},"tokens":null}`)
	if err != nil || value.Message.Value != nil || value.Tokens != nil {
		t.Fatalf("%+v %v", value, err)
	}
	value, err = provider.DecodeUntagged[report](`{"message":"slow down","tokens":12}`)
	if err != nil || value.Message.Value == nil || *value.Message.Value != "slow down" || value.Tokens == nil ||
		*value.Tokens != 12 {
		t.Fatalf("%+v %v", value, err)
	}
	for _, text := range []string{`{"tokens":12.5}`, `{"tokens":"10"}`, `{"tokens":-1}`} {
		if _, err := provider.DecodeUntagged[report](text); err == nil {
			t.Fatalf("accepted %s", text)
		}
	}
}

func TestSecretSingleLineAndRedaction(t *testing.T) {
	for _, tc := range []struct {
		text string
		err  error
	}{{"", provider.ErrSecretEmpty}, {"sk-1\nx", provider.ErrSecretControl}, {"x\u0085", provider.ErrSecretControl}} {
		_, err := provider.NewSecret(tc.text)
		if !errors.Is(err, tc.err) {
			t.Fatalf("%v", err)
		}
	}
	secret, err := provider.NewSecret("sk-ant-123")
	if err != nil {
		t.Fatal(err)
	}
	requireEqual(t, fmt.Sprintf("%#v %s %q", secret, secret, secret), "Secret(..) Secret(..) Secret(..)")
	requireEqual(t, secret.Expose(), "sk-ant-123")
	requireEqual(t, fmt.Sprintf("%v %v", secret.HeaderValue(), secret.Bearer()), "Secret(..) Secret(..)")
	decoded, err := provider.DecodeSecret([]byte(`"sk-ant-123"`))
	if err != nil {
		t.Fatal(err)
	}
	requireEqual(t, decoded, secret)
	if _, err := provider.DecodeSecret([]byte(`""`)); err == nil {
		t.Fatal("accepted empty credential")
	}
}
