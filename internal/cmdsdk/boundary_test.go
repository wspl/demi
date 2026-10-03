package cmdsdk

import (
	"bytes"
	"context"
	"encoding/binary"
	"encoding/json"
	"errors"
	"io"
	"os"
	"reflect"
	"strings"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/contract"
)

// This suite also exercises commandwire at the consumer boundary.
func TestDecodesEveryFragmentationBoundaryAndPreservesBinary(t *testing.T) {
	expected := []commandwire.Record{
		commandwire.InputPull{},
		commandwire.Stdout{0, 255, 13, 10},
		commandwire.Stderr("diagnostic"),
		commandwire.Completed{Completion: commandwire.Completion{ExitCode: 7}},
	}
	var wire []byte
	for _, r := range expected {
		b, err := commandwire.EncodeRecord(r)
		must(t, err)
		wire = append(wire, b...)
	}
	for size := 1; size <= len(wire); size++ {
		var d commandwire.RecordDecoder
		var got []commandwire.Record
		for offset := 0; offset < len(wire); offset += size {
			b := wire[offset:min(offset+size, len(wire))]
			for len(b) > 0 {
				r, n, err := d.Decode(b)
				must(t, err)
				b = b[n:]
				if r != nil {
					got = append(got, r)
				}
			}
		}
		must(t, d.Finish())
		if !reflect.DeepEqual(got, expected) {
			t.Fatalf("fragment size %d", size)
		}
	}
}

func TestRejectsOversizedHeaderBeforePayloadArrives(t *testing.T) {
	b := []byte{1, 0, 0, 0, 0}
	binary.BigEndian.PutUint32(b[1:], commandwire.MaxRecordBytes+1)
	var d commandwire.RecordDecoder
	_, _, err := d.Decode(b)
	if !errors.Is(err, commandwire.ErrTooLarge) {
		t.Fatal(err)
	}
}

func TestRequiresCompletionAndRejectsTrailingRecords(t *testing.T) {
	var d commandwire.RecordDecoder
	b, err := commandwire.EncodeRecord(commandwire.Stdout("partial"))
	must(t, err)
	_, _, err = d.Decode(b)
	must(t, err)
	if !errors.Is(d.Finish(), commandwire.ErrIncomplete) {
		t.Fatal("accepted missing completion")
	}
	d = commandwire.RecordDecoder{}
	b, err = commandwire.EncodeRecord(commandwire.Completed{})
	must(t, err)
	_, _, err = d.Decode(b)
	must(t, err)
	_, _, err = d.Decode(b)
	if !errors.Is(err, commandwire.ErrAfterCompletion) {
		t.Fatal(err)
	}
}

func TestRejectsInvalidCompletionExitCodesAtWireBoundary(t *testing.T) {
	for _, s := range []string{`{"exitCode":-1}`, `{"exitCode":256}`, `{"exitCode":0,"error":null}`} {
		b := make([]byte, 5)
		b[0] = 3
		binary.BigEndian.PutUint32(b[1:], uint32(len(s)))
		b = append(b, []byte(s)...)
		var d commandwire.RecordDecoder
		if _, _, err := d.Decode(b); err == nil {
			t.Fatal(s)
		}
	}
}

func TestDescriptorValidatesAndHashesToRecordedDigest(t *testing.T) {
	b, err := os.ReadFile("testdata/package.json")
	must(t, err)
	// The fixture envelope is test data; the descriptor enters through its generated decoder.
	var envelope map[string]json.RawMessage
	must(t, json.Unmarshal(b, &envelope))
	p, err := commandwire.DecodePackageDescriptor(envelope["descriptor"])
	must(t, err)
	digest, err := p.Digest()
	must(t, err)
	var want string
	must(t, json.Unmarshal(envelope["digest"], &want))
	if digest != want {
		t.Fatalf("%s != %s", digest, want)
	}
	p.Operations = []string{"same", "same"}
	if decodeDescriptor(t, p) == nil {
		t.Fatal("duplicate operations")
	}
}

func TestConversationStatusChecksConversationIdentity(t *testing.T) {
	for _, s := range []string{`{"conversations":[]}`, `{"conversations":["conversation"]}`} {
		_, err := commandwire.DecodeConversationStatus([]byte(s))
		must(t, err)
	}
	if _, err := commandwire.DecodeConversationStatus([]byte(`{"conversations":[""]}`)); err == nil {
		t.Fatal("empty identity")
	}
}

func TestDescriptorRetryAndPathResolution(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		start := time.Now()
		tries := 0
		v, err := Retry(t.Context(), func() (int, error) {
			tries++
			if tries < 7 {
				return 0, Exhaustion()
			}
			return 42, nil
		})
		must(t, err)
		if v != 42 || time.Since(start) != 255*time.Millisecond {
			t.Fatalf("retry value %d duration %s", v, time.Since(start))
		}
	})
	for _, path := range []string{"", "bad\x00path"} {
		if _, err := Resolve("/work", path); err == nil {
			t.Fatal("invalid path")
		}
	}
	got, err := Resolve("/work", "link/../file")
	must(t, err)
	if got != "/work/link/../file" {
		t.Fatal(got)
	}
	for _, args := range [][]string{nil, {"--help"}, {CommandService, "extra"}} {
		if err := CheckLaunch(args); err == nil {
			t.Fatal(args)
		}
	}
	must(t, CheckLaunch([]string{CommandService}))
}

func TestExchangeStopsInputWhenServiceCompletes(t *testing.T) {
	c, _ := connected(t, fixture{})
	i, o, err := c.Invoke(t.Context(), invocation("short"))
	must(t, err)
	source := &waitingSource{}
	sink := &capture{}
	_, err = (Exchange{i, o}).Run(t.Context(), source, sink)
	must(t, err)
	if source.called {
		t.Fatal("read unrequested stdin")
	}
}

type waitingSource struct{ called bool }

func (s *waitingSource) Next(ctx context.Context) ([]byte, error) {
	s.called = true
	<-ctx.Done()
	return nil, ctx.Err()
}

func TestMetadataAndInputSizeRejectBeforeAllocation(t *testing.T) {
	for _, limit := range []uint32{commandwire.MaxRecordBytes, commandwire.MaxMetadataBytes} {
		var b [4]byte
		binary.BigEndian.PutUint32(b[:], limit+1)
		_, err := readChunk(bytes.NewReader(b[:]), limit)
		if !errors.Is(err, commandwire.ErrTooLarge) {
			t.Fatal(err)
		}
	}
	_, err := readChunk(bytes.NewReader([]byte{0, 0}), 10)
	if !errors.Is(err, io.ErrUnexpectedEOF) {
		t.Fatal(err)
	}
}

func TestPackageDecodingEnforcesValueConstraints(t *testing.T) {
	b, err := os.ReadFile("testdata/package.json")
	must(t, err)
	var envelope map[string]json.RawMessage
	must(t, json.Unmarshal(b, &envelope))
	for _, tc := range []struct {
		path  []string
		value any
	}{
		{[]string{"id"}, "Invalid package"},
		{[]string{"version"}, ""},
		{[]string{"protocolVersion"}, 2},
		{[]string{"operations"}, []string{}},
		{[]string{"operations"}, []string{""}},
		{[]string{"operations"}, []string{"same", "same"}},
		{[]string{"targets", "aarch64-apple-darwin", "sha256"}, strings.Repeat("g", 64)},
		{[]string{"targets", "aarch64-apple-darwin", "size"}, 0},
		{[]string{"targets", "aarch64-apple-darwin", "size"}, 1.5},
		{[]string{"targets", "aarch64-apple-darwin", "size"}, uint64(9007199254740992)},
	} {
		var value map[string]any
		must(t, json.Unmarshal(envelope["descriptor"], &value))
		target := value
		for _, key := range tc.path[:len(tc.path)-1] {
			target = target[key].(map[string]any)
		}
		target[tc.path[len(tc.path)-1]] = tc.value
		invalid, err := json.Marshal(value)
		must(t, err)
		if _, err = commandwire.DecodePackageDescriptor(invalid); err == nil {
			t.Fatalf("accepted %v", tc.path)
		}
	}
	p, err := commandwire.DecodePackageDescriptor(envelope["descriptor"])
	must(t, err)
	artifact := p.Targets["aarch64-apple-darwin"]
	delete(p.Targets, "aarch64-apple-darwin")
	must(t, decodeDescriptor(t, p))
	p.Targets["riscv64-unknown-linux-musl"] = artifact
	if decodeDescriptor(t, p) == nil {
		t.Fatal("unknown target")
	}
}

func TestArtifactURLIsHTTPOrHTTPSWithoutCredentials(t *testing.T) {
	for _, tc := range []struct {
		url   string
		valid bool
	}{
		{"https://demi-native.s3.amazonaws.com/native/blobs/a", true},
		{"http://192.168.5.2:3271/native-artifacts/a", true},
		{"https://user:secret@demi-native.s3.amazonaws.com/native/blobs/a", false},
		{"ftp://192.168.5.2/native-artifacts/a", false},
		{"file:///native-artifacts/a", false},
		{"native-artifacts/a", false},
	} {
		if err := (&commandwire.ArtifactURL{URL: tc.url}).Validate(); (err == nil) != tc.valid {
			t.Fatalf("%s: %v", tc.url, err)
		}
	}
}

func TestInvocationDecodingChecksNestedValuesAndOptionalNulls(t *testing.T) {
	c, _ := connected(t, fixture{})
	valid, err := contract.EncodeJSON(invocation("short"))
	must(t, err)
	for _, tc := range []struct{ old, new string }{
		{
			`"context":{"conversation":"one","caller":{"kind":"agent","number":1},` +
				`"locale":{"timeZone":"UTC","languages":["en-US"]}},`,
			``,
		},
		{`"operation":"short"`, `"operation":""`},
		{`"invocationId":"short"`, `"invocationId":""`},
		{`"conversation":"one"`, `"conversation":""`},
		{`"number":1`, `"number":-1`},
		{`"kind":"agent","number":1`, `"kind":"agent"`},
		{`"kind":"agent"`, `"kind":"user"`},
		{`"kind":"agent"`, `"kind":"system"`},
		{`"languages":["en-US"]`, `"languages":[]`},
		{`"timeZone":"UTC"`, `"timeZone":""`},
		{`"languages":["en-US"]`, `"languages":[` + strings.TrimSuffix(strings.Repeat(`"en",`, 17), ",") + `]`},
		{`"locale":{"timeZone":"UTC","languages":["en-US"]}`, `"ignored":{}`},
		{`"args":{}`, `"args":[]`},
		{`"cwd":"/tmp"`, `"cwd":"bad\u0000path"`},
		{`"env":{}`, `"env":{"A=B":"value"}`},
		{`"env":{}`, `"env":{"A":"bad\u0000value"}`},
		{`"args":{}`, `"args":{},"resource":{"id":"old","kind":"browser"}`},
		{`"args":{}`, `"args":{},"json":null`},
	} {
		body := strings.Replace(string(valid), tc.old, tc.new, 1)
		if body == string(valid) {
			t.Fatalf("mutation missed %s", tc.old)
		}
		b := make([]byte, 4)
		binary.BigEndian.PutUint32(b, uint32(len(body)))
		b = append(b, body...)
		_, _, err := c.invokeAt(t.Context(), commandwire.InvokePath, b, true)
		if err == nil || !strings.Contains(err.Error(), "service rejected HTTP request with status 400") {
			t.Fatalf("accepted %s: %v", body, err)
		}
	}
	user := invocation("short")
	user.Context.Caller = &commandwire.UserCaller{}
	_, o, err := c.Invoke(t.Context(), user)
	must(t, err)
	completed(t, o)
}

// decodeDescriptor observes descriptor validation at the package input boundary.
func decodeDescriptor(t *testing.T, p commandwire.PackageDescriptor) error {
	t.Helper()
	type unchecked commandwire.PackageDescriptor
	b, err := contract.EncodeJSON(unchecked(p))
	must(t, err)
	_, err = commandwire.DecodePackageDescriptor(b)
	return err
}
