package commandproto_test

import (
	"bytes"
	"encoding/binary"
	"errors"
	"reflect"
	"testing"

	"github.com/wspl/demi/internal/commandproto"
)

func TestFragmentedResponse(t *testing.T) {
	records := []commandproto.Record{
		commandproto.Stdout("hello"),
		commandproto.Stderr{0, 255},
		commandproto.InputPull{},
		commandproto.Completed{
			Completion: commandproto.Completion{
				ExitCode: 7,
				Error:    &commandproto.CommandError{Code: "failed", Message: "why"},
			},
		},
	}
	var wire []byte
	for _, r := range records {
		data, err := commandproto.EncodeRecord(r)
		if err != nil {
			t.Fatal(err)
		}
		wire = append(wire, data...)
	}
	for split := 0; split <= len(wire); split++ {
		var decoder commandproto.RecordDecoder
		var got []commandproto.Record
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
		if _, _, err := decoder.Decode([]byte{1}); !errors.Is(err, commandproto.ErrAfterCompletion) {
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
		{"unknown kind", []byte{5, 0, 0, 0, 0}, commandproto.ErrUnknownRecord},
		{"oversized header", []byte{1, 255, 255, 255, 255}, commandproto.ErrTooLarge},
		{"pull payload", []byte{4, 0, 0, 0, 1, 1}, commandproto.ErrInvalidRecord},
	} {
		t.Run(tc.name, func(t *testing.T) {
			var d commandproto.RecordDecoder
			if _, _, err := d.Decode(tc.frame); !errors.Is(err, tc.want) {
				t.Fatalf("got %v; want %v", err, tc.want)
			}
		})
	}
	complete, err := commandproto.EncodeRecord(commandproto.Completed{Completion: commandproto.Completion{ExitCode: 0}})
	if err != nil {
		t.Fatal(err)
	}
	for end := 0; end < len(complete); end++ {
		var d commandproto.RecordDecoder
		if _, _, err := d.Decode(complete[:end]); err != nil {
			t.Fatal(err)
		}
		if err := d.Finish(); !errors.Is(err, commandproto.ErrIncomplete) {
			t.Fatalf("prefix %d: %v", end, err)
		}
	}
	for _, payload := range []string{`{"exitCode":-1}`, `{"exitCode":256}`, `{"exitCode":0,"error":null}`} {
		frame := make([]byte, 5+len(payload))
		frame[0] = 3
		binary.BigEndian.PutUint32(frame[1:], uint32(len(payload)))
		copy(frame[5:], payload)
		var d commandproto.RecordDecoder
		if _, _, err := d.Decode(frame); err == nil {
			t.Fatalf("accepted invalid completion %s", payload)
		}
		if err := d.Finish(); !errors.Is(err, commandproto.ErrIncomplete) {
			t.Fatalf("invalid completion %s ended stream", payload)
		}
	}
}

func TestBoundedFrames(t *testing.T) {
	data := bytes.Repeat([]byte{42}, commandproto.MaxRecordBytes)
	input, err := commandproto.EncodeInput(data)
	if err != nil || !bytes.Equal(input[4:], data) || binary.BigEndian.Uint32(input[:4]) != uint32(len(data)) {
		t.Fatalf("input framing: %v", err)
	}
	record, err := commandproto.EncodeRecord(commandproto.Stdout(data))
	if err != nil || record[0] != 1 || !bytes.Equal(record[5:], data) {
		t.Fatalf("record framing: %v", err)
	}
	data = append(data, 0)
	if _, err := commandproto.EncodeInput(data); !errors.Is(err, commandproto.ErrTooLarge) {
		t.Fatal(err)
	}
	if _, err := commandproto.EncodeRecord(commandproto.Stdout(data)); !errors.Is(err, commandproto.ErrTooLarge) {
		t.Fatal(err)
	}
	metadata := commandproto.LocalInvocation{
		Operation:    "echo",
		InvocationID: "1",
		Args:         []byte(`{}`),
		Cwd:          "/tmp",
		Env:          map[string]string{},
	}
	framed, err := commandproto.EncodeMetadata(metadata)
	if err != nil {
		t.Fatal(err)
	}
	if int(binary.BigEndian.Uint32(framed[:4])) != len(framed)-4 {
		t.Fatal("incorrect metadata length")
	}
	if _, err := commandproto.DecodeLocalInvocation(framed[4:]); err != nil {
		t.Fatal(err)
	}
	metadata.Operation = string(bytes.Repeat([]byte{'x'}, commandproto.MaxMetadataBytes))
	if _, err := commandproto.EncodeMetadata(metadata); !errors.Is(err, commandproto.ErrTooLarge) {
		t.Fatal(err)
	}
}

// unrelatedMetadata models another package's value that happens to validate.
type unrelatedMetadata struct{}

func (unrelatedMetadata) Validate() error {
	return nil
}

func TestMetadataAdmission(t *testing.T) {
	if reflect.TypeOf(commandproto.EncodeMetadata).In(0) != reflect.TypeFor[commandproto.Metadata]() {
		t.Fatal("EncodeMetadata must take the sealed Metadata interface")
	}
	for _, tc := range []struct {
		name    string
		value   any
		allowed bool
	}{
		{"native", commandproto.Invocation{}, true},
		{"local", commandproto.LocalInvocation{}, true},
		{"native pointer", &commandproto.Invocation{}, true},
		{"local pointer", &commandproto.LocalInvocation{}, true},
		{"foreign validator", unrelatedMetadata{}, false},
		{"stream open", commandproto.StreamOpen{}, false},
		{"conversation release", &commandproto.ConversationRelease{}, false},
		{"conversation query", &commandproto.ConversationQuery{}, false},
		{"completion", commandproto.Completion{}, false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			_, ok := tc.value.(commandproto.Metadata)
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
		{"stream open", commandproto.StreamOpen{}.Encode, `{}`},
		{"release", func() ([]byte, error) {
			return commandproto.EncodeConversationRequest(&commandproto.ConversationRelease{Conversation: "conv_1"})
		}, `{"operation":"release","conversation":"conv_1"}`},
		{
			"status",
			func() ([]byte, error) {
				return commandproto.EncodeConversationRequest(&commandproto.ConversationQuery{})
			},
			`{"operation":"status"}`,
		},
	} {
		t.Run(tc.name, func(t *testing.T) {
			frame, err := tc.encode()
			if err != nil {
				t.Fatal(err)
			}
			if len(frame) < 4 || int(binary.BigEndian.Uint32(frame[:4])) != len(frame)-4 ||
				string(frame[4:]) != tc.want {
				t.Fatalf("metadata frame = %q; want length-prefixed %s", frame, tc.want)
			}
		})
	}
	if _, err := commandproto.EncodeConversationRequest(
		&commandproto.ConversationRelease{Conversation: "../invalid"},
	); err == nil {
		t.Fatal("invalid conversation release was framed")
	}
}

func TestFramedJSONLeavesHTMLCharactersUnescaped(t *testing.T) {
	text := "<&>\u2028\u2029"
	for _, tc := range []struct {
		name   string
		encode func() ([]byte, error)
		prefix int
		want   string
	}{
		{"completion", func() ([]byte, error) {
			return commandproto.EncodeRecord(commandproto.Completed{
				Completion: commandproto.Completion{
					ExitCode: 1,
					Error: &commandproto.CommandError{
						Code:    "failed",
						Message: text,
					},
				},
			})
		}, 5, `{"exitCode":1,"error":{"code":"failed","message":"` + text + `"}}`},
		{"metadata", func() ([]byte, error) {
			return commandproto.EncodeMetadata(commandproto.LocalInvocation{
				Operation:    text,
				InvocationID: "i",
				Args:         []byte(`{}`),
				Cwd:          "/tmp",
				Env:          map[string]string{},
			})
		}, 4, `{"operation":"` + text + `","invocationId":"i","args":{},"cwd":"/tmp","env":{}}`},
	} {
		t.Run(tc.name, func(t *testing.T) {
			frame, err := tc.encode()
			if err != nil {
				t.Fatal(err)
			}
			if len(frame) < tc.prefix {
				t.Fatalf("short frame: %x", frame)
			}
			if tc.prefix == 5 && frame[0] != 3 {
				t.Fatalf("record kind = %d; want completion", frame[0])
			}
			if got := binary.BigEndian.Uint32(frame[tc.prefix-4 : tc.prefix]); int(got) != len(tc.want) {
				t.Fatalf("payload length = %d; want %d", got, len(tc.want))
			}
			if got := string(frame[tc.prefix:]); got != tc.want {
				t.Fatalf("payload = %q; want unescaped bytes %q", got, tc.want)
			}
		})
	}
}
