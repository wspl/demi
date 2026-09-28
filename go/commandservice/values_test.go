package commandservice_test

import (
	"bytes"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"errors"
	"strings"
	"testing"

	cs "github.com/wspl/demi/go/commandservice"
)

// Each table checks a wire constraint at Decode, without IO; total budget one second.
func rejectValues[T cs.WireValue](t *testing.T, values []string) {
	t.Helper()
	for _, value := range values {
		if _, err := cs.Decode[T]([]byte(value)); err == nil {
			t.Errorf("accepted %s", value)
		}
	}
}

func TestWireValueConstraints(t *testing.T) {
	rejectValues[cs.PackageDescriptor](t, []string{
		`{"id":"single","version":"v","protocolVersion":1,"operations":["a"],"targets":{}}`,
		`{"id":"a.b","version":"","protocolVersion":1,"operations":["a"],"targets":{}}`,
		`{"id":"a.b","version":"v","protocolVersion":2,"operations":["a"],"targets":{}}`,
		`{"id":"a.b","version":"v","protocolVersion":1,"operations":["a","a"],"targets":{}}`,
		`{"id":"a.b","version":"v","protocolVersion":1,"operations":[],"targets":{}}`,
		`{"id":"a.b","version":"v","protocolVersion":1,"operations":["a"],"targets":{"other":{"sha256":"` + strings.Repeat("a", 64) + `","size":1}}}`,
	})
	for _, value := range []string{
		`{"sha256":"abc","size":1}`,
		`{"sha256":"` + strings.Repeat("A", 64) + `","size":1}`,
		`{"sha256":"` + strings.Repeat("a", 64) + `","size":0}`,
		`{"sha256":"` + strings.Repeat("a", 64) + `","size":9007199254740992}`,
	} {
		rejectValues[cs.PackageDescriptor](
			t,
			[]string{
				`{"id":"a.b","version":"v","protocolVersion":1,"operations":["a"],"targets":{"x86_64-unknown-linux-musl":` + value + `}}`,
			},
		)
	}
	for _, value := range []string{
		`{"kind":"agent"}`, `{"kind":"user","number":1}`, `{"kind":"agent","number":null}`,
		`{"kind":"other"}`, `{"kind":"agent","number":-1}`,
	} {
		rejectValues[cs.Invocation](
			t,
			[]string{
				`{"operation":"echo","invocationId":"i","cwd":"/tmp","env":{},"args":{},"context":{"conversation":"c","locale":{"timeZone":"UTC","languages":["en"]},"caller":` + value + `}}`,
			},
		)
	}
	for _, value := range []string{
		`{"timeZone":"","languages":["en"]}`, `{"timeZone":"UTC","languages":[]}`,
		`{"timeZone":"UTC","languages":[""]}`,
		`{"timeZone":"` + strings.Repeat("x", 65) + `","languages":["en"]}`,
		`{"timeZone":"UTC","languages":["` + strings.Repeat("x", 65) + `"]}`,
		`{"timeZone":"UTC","languages":[` + strings.TrimSuffix(strings.Repeat(`"en",`, 17), ",") + `]}`,
	} {
		rejectValues[cs.Invocation](
			t,
			[]string{
				`{"operation":"echo","invocationId":"i","cwd":"/tmp","env":{},"args":{},"context":{"conversation":"c","caller":{"kind":"user"},"locale":` + value + `}}`,
			},
		)
	}
	value := invocation("echo")
	value.Context.Locale.TimeZone = strings.Repeat("😀", 64)
	if _, err := cs.Encode(value); err != nil {
		t.Fatal(err)
	}
	rejectValues[cs.NumbersRequest](t, []string{
		`{"id":0,"conversation":"c","sequence":"tab","count":0}`,
		`{"id":0,"conversation":"c","sequence":"tab","count":17}`,
		`{"id":0,"conversation":"../c","sequence":"tab","count":1}`,
		`{"id":0,"conversation":"c","sequence":"other","count":1}`,
	})
	rejectValues[cs.NumbersAnswer](t, []string{
		`{"id":0}`, `{"id":0,"first":1,"error":"failed"}`, `{"id":0,"first":0}`,
		`{"id":0,"error":""}`, `{"id":0,"first":null}`,
	})
	rejectValues[*cs.EditContext](
		t,
		[]string{`{"directory":"relative","lock":"/lock"}`, `{"directory":"/dir","lock":"relative"}`},
	)
	rejectValues[*cs.ArtifactURL](t, []string{`{"url":"https://example.com:99999/a"}`, `{"url":"https://u:p@example.com"}`})
	rejectValues[*cs.NumbersAnswer](t, []string{`{"id":0}`})
	if _, err := cs.Decode[cs.ArtifactURL]([]byte(`{"url":"https:///missing"}`)); err != nil {
		t.Fatal(err)
	}
	rejectValues[cs.EditJournal](t, []string{
		`{"files":[],"bytesCopied":67108865,"nextSegment":0,"filesTruncated":false}`,
		`{"files":[],"bytesCopied":0,"nextSegment":1001,"filesTruncated":false}`,
		`{"files":[{"path":"","kind":"added","edits":[]}],"bytesCopied":0,"nextSegment":0,"filesTruncated":false}`,
		`{"files":[{"path":"a","kind":"other","edits":[]}],"bytesCopied":0,"nextSegment":0,"filesTruncated":false}`,
		`{"files":[{"path":"a","kind":"added","edits":[{"original":null}]}],"bytesCopied":0,"nextSegment":0,"filesTruncated":false}`,
	})
	for _, journal := range []cs.EditJournal{
		{Files: make([]cs.EditFile, 501)},
		{Files: []cs.EditFile{{Path: "a", Kind: cs.Added, Edits: make([]cs.EditCopies, 1001)}}},
	} {
		// Populate valid files so each case isolates the array bound.
		for index := range journal.Files {
			if journal.Files[index].Path == "" {
				journal.Files[index] = cs.EditFile{Path: "a", Kind: cs.Added}
			}
		}
		if _, err := cs.Encode(journal); err == nil {
			t.Error("accepted oversized journal")
		}
	}
}

func TestOpaqueArgumentsAndStandaloneSizes(t *testing.T) {
	value := invocation("echo")
	value.Args = jsontext.Value(`{"integer":9007199254740993,"nested":{"z":true,"a":[null,18446744073709551615]}}`)
	value.Env = map[string]string{"z": "last", "a": "first"}
	encoded, err := cs.Encode(value)
	if err != nil {
		t.Fatal(err)
	}
	decoded, err := cs.Decode[cs.Invocation](encoded)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(decoded.Args, value.Args) {
		t.Fatalf("changed args: %s", decoded.Args)
	}
	if !bytes.Contains(encoded, []byte(`"env":{"a":"first","z":"last"}`)) {
		t.Fatalf("unordered environment: %s", encoded)
	}
	descriptor := cs.PackageDescriptor{
		ID: "a.b", Version: strings.Repeat("x", cs.MaxMetadataBytes), ProtocolVersion: cs.Version,
		Operations: []string{"echo"}, Targets: map[string]cs.PackageArtifact{},
	}
	if _, err = descriptor.Digest(); err != nil {
		t.Fatal(err)
	}
	data, err := json.Marshal(descriptor)
	if err != nil {
		t.Fatal(err)
	}
	if _, err = cs.Decode[cs.PackageDescriptor](data); err != nil {
		t.Fatal(err)
	}
	value.Args = jsontext.Value(`{"large":"` + strings.Repeat("x", cs.MaxMetadataBytes) + `"}`)
	if _, err = cs.EncodeMetadata(value); err != cs.ErrTooLarge {
		t.Fatal(err)
	}
}

// Duplicate names must be refused before schema validation can hide an earlier value.
func TestDuplicateMembersAreRejected(t *testing.T) {
	value := invocation("echo")
	encoded, err := cs.Encode(value)
	if err != nil {
		t.Fatal(err)
	}
	for _, test := range []struct{ name, field, replacement string }{
		{"environment", `"env":{}`, `"env":{"x":null,"x":"valid"}`},
		{"arguments", `"args":{}`, `"args":{"label":{"a":1,"a":2}}`},
		{"struct", `"operation":"echo"`, `"operation":"echo","operation":"first"`},
	} {
		t.Run(test.name, func(t *testing.T) {
			data := bytes.Replace(encoded, []byte(test.field), []byte(test.replacement), 1)
			_, err := cs.Decode[cs.Invocation](data)
			var invalid *cs.InvalidError
			if !errors.As(err, &invalid) || invalid.Field != "JSON" {
				t.Fatalf("duplicate reached schema validation: %v", err)
			}
		})
	}
	value.Args = []byte(`{"a":1,"a":2}`)
	if _, err := cs.Encode(value); err == nil {
		t.Fatal("encoded duplicate opaque member")
	}
}
