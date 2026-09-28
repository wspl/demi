package commandservice_test

import (
	"bytes"
	"encoding/binary"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"errors"
	"io"
	"os"
	"strings"
	"testing"

	cs "github.com/wspl/demi/go/commandservice"
)

// Wire tables are in-memory checks, budget <1 second, with no external services.
func TestWireContracts(t *testing.T) {
	b, err := os.ReadFile("../../crates/command-service/tests/command_service/fixtures/package.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Descriptor jsontext.Value `json:"descriptor"`
		Digest     string         `json:"digest"`
	}
	if err := json.Unmarshal(b, &fixture); err != nil {
		t.Fatal(err)
	}
	descriptor, err := cs.Decode[cs.PackageDescriptor](fixture.Descriptor)
	if err != nil {
		t.Fatal(err)
	}
	digest, err := descriptor.Digest()
	if err != nil || digest != fixture.Digest {
		t.Fatalf("digest %s: %v", digest, err)
	}
	for _, s := range []string{
		`{}`,
		`{"exitCode":-1}`,
		`{"exitCode":256}`,
		`{"exitCode":0,"error":null}`,
		`{"exitCode":0,"extra":true}`,
	} {
		if _, err := cs.Decode[cs.Completion]([]byte(s)); !errors.Is(err, cs.ErrInvalid) {
			t.Errorf("accepted %s: %v", s, err)
		}
	}
	for _, s := range []string{
		`{"operation":"status","conversation":"x"}`,
		`{"operation":"release","conversation":"../x"}`,
		`{"operation":"release"}`,
	} {
		if _, err := cs.Decode[cs.ConversationRequest]([]byte(s)); err == nil {
			t.Errorf("accepted %s", s)
		}
	}
	for _, u := range []string{"file:///tmp/a", "https://u:p@example.com/a", ""} {
		if _, err := cs.Encode(cs.ArtifactURL{URL: u}); err == nil {
			t.Errorf("accepted URL %s", u)
		}
	}
	if _, err := cs.Encode(cs.ArtifactURL{URL: "https://example.com/a?signature=x"}); err != nil {
		t.Fatal(err)
	}
	v := invocation("echo")
	b, err = cs.Encode(v)
	if err != nil {
		t.Fatal(err)
	}
	for _, bad := range []string{
		strings.Replace(string(b), `"args":{}`, `"args":null`, 1),
		strings.Replace(string(b), `"env":{}`, `"env":{"a=b":"x"}`, 1),
		strings.TrimSuffix(string(b), "}") + `,"edits":null}`,
		strings.TrimSuffix(string(b), "}") + `,"json":null}`,
	} {
		if _, err := cs.Decode[cs.Invocation]([]byte(bad)); err == nil {
			t.Errorf("accepted %s", bad)
		}
	}
	var raw map[string]jsontext.Value
	if err = json.Unmarshal(b, &raw); err != nil {
		t.Fatal(err)
	}
	for key := range raw {
		copy := make(map[string]jsontext.Value)
		for k, v := range raw {
			if k != key {
				copy[k] = v
			}
		}
		missing, err := json.Marshal(copy)
		if err != nil {
			t.Fatal(err)
		}
		if _, err := cs.Decode[cs.Invocation](missing); err == nil {
			t.Errorf("accepted missing %s", key)
		}
	}
}

func TestFraming(t *testing.T) {
	payload := []byte{0, 255, 1, 0, 128}
	a, err := (cs.Record{Kind: cs.Stdout, Data: payload}).Encode()
	if err != nil {
		t.Fatal(err)
	}
	end, err := (cs.Record{Kind: cs.Completed, Completion: cs.Completion{ExitCode: 17}}).Encode()
	if err != nil {
		t.Fatal(err)
	}
	wire := append(a, end...)
	for split := 0; split <= len(wire); split++ {
		d := cs.RecordDecoder{Reader: io.MultiReader(bytes.NewReader(wire[:split]), bytes.NewReader(wire[split:]))}
		r, err := d.Next()
		if err != nil || !bytes.Equal(r.Data, payload) {
			t.Fatal(split, r, err)
		}
		r, err = d.Next()
		if err != nil || r.Completion.ExitCode != 17 {
			t.Fatal(split, r, err)
		}
		if _, err = d.Next(); err != io.EOF {
			t.Fatal(err)
		}
	}
	for _, test := range []struct {
		wire []byte
		err  error
	}{
		{a, cs.ErrIncomplete},
		{append(append([]byte{}, end...), 0), cs.ErrAfterCompletion},
		{[]byte{9, 0, 0, 0, 0}, cs.ErrUnknownRecord},
	} {
		d := cs.RecordDecoder{Reader: bytes.NewReader(test.wire)}
		for {
			_, err := d.Next()
			if err != nil {
				if !errors.Is(err, test.err) {
					t.Fatal(err, test.err)
				}
				break
			}
		}
	}
	head := make([]byte, 5)
	head[0] = 1
	binary.BigEndian.PutUint32(head[1:], cs.MaxRecordBytes+1)
	d := cs.RecordDecoder{Reader: bytes.NewReader(head)}
	if _, err := d.Next(); !errors.Is(err, cs.ErrTooLarge) {
		t.Fatal(err)
	}
}
