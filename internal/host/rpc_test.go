package host_test

import (
	"context"
	"errors"
	"reflect"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/host/hosttest"
	"go.uber.org/goleak"
)

func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }

type answerTransport struct {
	response host.PortResponse
	err      error
}

func (a answerTransport) Request(context.Context, host.PortRequest) (host.PortResponse, error) {
	return a.response, a.err
}

func TestRPCPortMessagesAndUnexpectedResponses(t *testing.T) {
	memory := hosttest.NewMemoryPort(nil)
	memory.FeedStdin([]byte("finite"))
	memory.FeedStdin([]byte{})
	memory.FeedLive([]byte("live"))
	port := memory.Port()
	if err := port.Stdout(t.Context(), []byte("out")); err != nil {
		t.Fatal(err)
	}
	if err := port.Stderr(t.Context(), []byte("err")); err != nil {
		t.Fatal(err)
	}
	if string(memory.Stdout()) != "out" || string(memory.Stderr()) != "err" {
		t.Fatal("output differs")
	}
	for _, want := range [][]byte{[]byte("finite"), {}, nil} {
		got, err := port.ReadStdin(t.Context())
		if err != nil || !reflect.DeepEqual(got, want) {
			t.Fatalf("stdin %q %v want %q", got, err, want)
		}
	}
	if got, err := port.ReadLiveStdin(t.Context()); err != nil || string(got) != "live" {
		t.Fatalf("live %q %v", got, err)
	}
	if got, err := port.ReadLiveStdin(t.Context()); err != nil || got != nil {
		t.Fatalf("live EOF %q %v", got, err)
	}
	wrong := host.NewRPCPort(answerTransport{response: &host.PortInput{}})
	err := wrong.Stdout(t.Context(), nil)
	var portErr *host.PortError
	if !errors.As(err, &portErr) || portErr.Kind != host.UnexpectedReply || portErr.Asked != "stdout" ||
		portErr.Answered != "input" {
		t.Fatalf("unexpected: %v", err)
	}
	wrong = host.NewRPCPort(answerTransport{response: &host.PortWritten{}})
	if _, err = wrong.ReadStdin(t.Context()); !errors.As(err, &portErr) || portErr.Answered != "written" {
		t.Fatalf("unexpected read: %v", err)
	}
	if _, err = wrong.Storage(t.Context(), &host.StorageRead{Key: "x"}); !errors.As(err, &portErr) {
		t.Fatalf("unexpected storage: %v", err)
	}
	failure := errors.New("transport failed")
	if err = host.NewRPCPort(answerTransport{err: failure}).Stderr(t.Context(), nil); !errors.Is(err, failure) {
		t.Fatalf("lost transport error: %v", err)
	}
}

func TestRPCWirePreservesStrictnessNullableValuesAndBytes(t *testing.T) {
	cases := []struct {
		wire   string
		decode func([]byte) (any, error)
	}{
		{`{"type":"stdout","bytes":"AP8="}`, func(b []byte) (any, error) { return host.DecodePortRequest(b) }},
		{`{"type":"input","bytes":null}`, func(b []byte) (any, error) { return host.DecodePortResponse(b) }},
		{
			`{"op":"write_if","key":"<>&\u2028","value":null}`,
			func(b []byte) (any, error) { return host.DecodeStorageOp(b) },
		},
		{
			`{"outcome":"value","value":{"x":9007199254740993},"revision":18446744073709551615}`,
			func(b []byte) (any, error) { return host.DecodeStorageReply(b) },
		},
	}
	for _, tc := range cases {
		value, err := tc.decode([]byte(tc.wire))
		if err != nil {
			t.Fatalf("decode %s: %v", tc.wire, err)
		}
		encoded, err := contract.EncodeJSON(value)
		if err != nil {
			t.Fatal(err)
		}
		again, err := tc.decode(encoded)
		if err != nil || !reflect.DeepEqual(value, again) {
			t.Fatalf("roundtrip %s: %v", encoded, err)
		}
	}
	for _, wire := range []string{
		`{"type":"input"}`,
		`{"type":"input","bytes":"" ,"extra":1}`,
		`{"type":"input","bytes":null,"bytes":null}`,
	} {
		if _, err := host.DecodePortResponse([]byte(wire)); err == nil {
			t.Fatalf("accepted %s", wire)
		}
	}
	for _, wire := range []string{
		`{"op":"write_if","key":"x"}`,
		`{"op":"write_if","key":"x","value":null,"expected":null}`,
		`{"op":"read","key":"x","extra":1}`,
	} {
		if _, err := host.DecodeStorageOp([]byte(wire)); err == nil {
			t.Fatalf("accepted %s", wire)
		}
	}
}

func TestInvocationRetainsArgumentOrderAndRequiresObject(t *testing.T) {
	value := invocation([]string{"demi", "todo", "add"}, `{"text":"<>&\u2028","count":2}`)
	if string(value.Args) != `{"text":"<>&\u2028","count":2}` {
		t.Fatalf("arguments reordered: %s", value.Args)
	}
	encoded, err := value.MarshalJSON()
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(encoded), `"args":{"text":"<>&`+"\u2028"+`","count":2}`) {
		t.Fatalf("wire spelling: %s", encoded)
	}
	for _, raw := range []string{`[]`, `null`, `"text"`, `4`} {
		// Use the decoder's raw argument spelling; the encoder escapes only quotes, backslashes and control characters.
		decoded, err := host.DecodeRPCInvocation(encoded)
		if err != nil {
			t.Fatal(err)
		}
		invalid := strings.Replace(string(encoded), string(decoded.Args), raw, 1)
		if _, err = host.DecodeRPCInvocation([]byte(invalid)); err == nil {
			t.Fatalf("accepted non-object args %s", invalid)
		}
	}
	missing := hosttest.NewMemoryPort(nil)
	reply, err := missing.Port().Storage(t.Context(), &host.StorageRead{Key: "absent"})
	if err != nil {
		t.Fatal(err)
	}
	wire, err := contract.EncodeJSON(reply)
	if err != nil || string(wire) != `{"outcome":"value","value":null,"revision":0}` {
		t.Fatalf("missing storage wire %s: %v", wire, err)
	}
}
