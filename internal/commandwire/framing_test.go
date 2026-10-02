package commandwire_test

import (
	"bytes"
	"encoding/binary"
	"errors"
	"reflect"
	"testing"

	"github.com/wspl/demi/internal/commandwire"
)

func TestFragmentedResponse(t *testing.T) {
	records := []commandwire.Record{commandwire.Stdout("hello"), commandwire.Stderr{0, 255}, commandwire.InputPull{}, commandwire.Completed{Completion: commandwire.Completion{ExitCode: 7, Error: &commandwire.CommandError{Code: "failed", Message: "why"}}}}
	var wire []byte
	for _, r := range records {
		data, err := commandwire.EncodeRecord(r)
		if err != nil {
			t.Fatal(err)
		}
		wire = append(wire, data...)
	}
	for split := 0; split <= len(wire); split++ {
		var decoder commandwire.RecordDecoder
		var got []commandwire.Record
		for _, part := range [][]byte{wire[:split], wire[split:]} {
			for len(part) > 0 {
				r, n, err := decoder.Decode(part)
				if err != nil {
					t.Fatal(err)
				}
				if n == 0 {
					t.Fatal("decoder made no progress")
				}
				part = part[n:]
				if r != nil {
					got = append(got, r)
				}
			}
		}
		if err := decoder.Finish(); err != nil {
			t.Fatal(err)
		}
		if !reflect.DeepEqual(got, records) {
			t.Fatalf("split %d: got %#v", split, got)
		}
		if _, _, err := decoder.Decode([]byte{1}); !errors.Is(err, commandwire.ErrAfterCompletion) {
			t.Fatalf("trailing byte: %v", err)
		}
	}
}

func TestResponseFailures(t *testing.T) {
	for _, tc := range []struct {
		name  string
		frame []byte
		want  error
	}{
		{"unknown kind", []byte{5, 0, 0, 0, 0}, commandwire.ErrUnknownRecord},
		{"oversized header", []byte{1, 255, 255, 255, 255}, commandwire.ErrTooLarge},
		{"pull payload", []byte{4, 0, 0, 0, 1, 1}, commandwire.ErrInvalidRecord},
	} {
		t.Run(tc.name, func(t *testing.T) {
			var d commandwire.RecordDecoder
			if _, _, err := d.Decode(tc.frame); !errors.Is(err, tc.want) {
				t.Fatalf("got %v; want %v", err, tc.want)
			}
		})
	}
	complete, err := commandwire.EncodeRecord(commandwire.Completed{Completion: commandwire.Completion{ExitCode: 0}})
	if err != nil {
		t.Fatal(err)
	}
	for end := 0; end < len(complete); end++ {
		var d commandwire.RecordDecoder
		if _, _, err := d.Decode(complete[:end]); err != nil {
			t.Fatal(err)
		}
		if err := d.Finish(); !errors.Is(err, commandwire.ErrIncomplete) {
			t.Fatalf("prefix %d: %v", end, err)
		}
	}
	payload := []byte(`{"exitCode":256}`)
	frame := make([]byte, 5+len(payload))
	frame[0] = 3
	binary.BigEndian.PutUint32(frame[1:], uint32(len(payload)))
	copy(frame[5:], payload)
	var d commandwire.RecordDecoder
	if _, _, err := d.Decode(frame); err == nil {
		t.Fatal("accepted invalid completion")
	}
	if err := d.Finish(); !errors.Is(err, commandwire.ErrIncomplete) {
		t.Fatal("invalid completion ended stream")
	}
}

func TestBoundedFrames(t *testing.T) {
	data := bytes.Repeat([]byte{42}, commandwire.MaxRecordBytes)
	input, err := commandwire.EncodeInput(data)
	if err != nil || !bytes.Equal(input[4:], data) || binary.BigEndian.Uint32(input[:4]) != uint32(len(data)) {
		t.Fatalf("input framing: %v", err)
	}
	record, err := commandwire.EncodeRecord(commandwire.Stdout(data))
	if err != nil || record[0] != 1 || !bytes.Equal(record[5:], data) {
		t.Fatalf("record framing: %v", err)
	}
	data = append(data, 0)
	if _, err := commandwire.EncodeInput(data); !errors.Is(err, commandwire.ErrTooLarge) {
		t.Fatal(err)
	}
	if _, err := commandwire.EncodeRecord(commandwire.Stdout(data)); !errors.Is(err, commandwire.ErrTooLarge) {
		t.Fatal(err)
	}
	metadata := commandwire.LocalInvocation{Operation: "echo", InvocationID: "1", Args: []byte(`{}`), Cwd: "/tmp", Env: map[string]string{}}
	framed, err := commandwire.EncodeMetadata(metadata)
	if err != nil {
		t.Fatal(err)
	}
	if int(binary.BigEndian.Uint32(framed[:4])) != len(framed)-4 {
		t.Fatal("incorrect metadata length")
	}
	if _, err := commandwire.DecodeLocalInvocation(framed[4:]); err != nil {
		t.Fatal(err)
	}
	metadata.Operation = string(bytes.Repeat([]byte{'x'}, commandwire.MaxMetadataBytes))
	if _, err := commandwire.EncodeMetadata(metadata); !errors.Is(err, commandwire.ErrTooLarge) {
		t.Fatal(err)
	}
}

// unrelatedMetadata models another package's value that happens to validate.
type unrelatedMetadata struct{}

func (unrelatedMetadata) Validate() error { return nil }

func TestMetadataAdmission(t *testing.T) {
	if reflect.TypeOf(commandwire.EncodeMetadata).In(0) != reflect.TypeFor[commandwire.Metadata]() {
		t.Fatal("EncodeMetadata must take the sealed Metadata interface")
	}
	for _, tc := range []struct {
		name    string
		value   any
		allowed bool
	}{
		{"native", commandwire.Invocation{}, true},
		{"local", commandwire.LocalInvocation{}, true},
		{"native pointer", &commandwire.Invocation{}, true},
		{"local pointer", &commandwire.LocalInvocation{}, true},
		{"foreign validator", unrelatedMetadata{}, false},
		{"stream open", commandwire.StreamOpen{}, false},
		{"conversation release", &commandwire.ConversationRelease{}, false},
		{"conversation query", &commandwire.ConversationQuery{}, false},
		{"completion", commandwire.Completion{}, false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			_, ok := tc.value.(commandwire.Metadata)
			if ok != tc.allowed {
				t.Fatalf("Metadata admission = %v; want %v", ok, tc.allowed)
			}
		})
	}
}

func TestLifecycleMetadataFraming(t *testing.T) {
	for _, tc := range []struct {
		name   string
		encode func() ([]byte, error)
		want   string
	}{
		{"stream open", commandwire.StreamOpen{}.Encode, `{}`},
		{"release", func() ([]byte, error) {
			return commandwire.EncodeConversationRequest(&commandwire.ConversationRelease{Conversation: "conv_1"})
		}, `{"operation":"release","conversation":"conv_1"}`},
		{"status", func() ([]byte, error) { return commandwire.EncodeConversationRequest(&commandwire.ConversationQuery{}) }, `{"operation":"status"}`},
	} {
		t.Run(tc.name, func(t *testing.T) {
			frame, err := tc.encode()
			if err != nil {
				t.Fatal(err)
			}
			if len(frame) < 4 || int(binary.BigEndian.Uint32(frame[:4])) != len(frame)-4 || string(frame[4:]) != tc.want {
				t.Fatalf("metadata frame = %q; want length-prefixed %s", frame, tc.want)
			}
		})
	}
	if _, err := commandwire.EncodeConversationRequest(&commandwire.ConversationRelease{Conversation: "../invalid"}); err == nil {
		t.Fatal("invalid conversation release was framed")
	}
}
