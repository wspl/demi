package cmdproto_test

import (
	"bytes"
	"encoding/binary"
	"errors"
	"reflect"
	"testing"

	"github.com/wspl/demi/internal/cmdproto"
)

func TestFragmentedResponse(t *testing.T) {
	records := []cmdproto.Record{
		cmdproto.Stdout("hello"),
		cmdproto.Stderr{0, 255},
		cmdproto.InputPull{},
		cmdproto.Completed{
			Completion: cmdproto.Completion{
				ExitCode: 7,
				Error:    &cmdproto.CommandError{Code: "failed", Message: "why"},
			},
		},
	}
	var wire []byte
	for _, r := range records {
		data, err := cmdproto.EncodeRecord(r)
		if err != nil {
			t.Fatal(err)
		}
		wire = append(wire, data...)
	}
	for split := 0; split <= len(wire); split++ {
		var decoder cmdproto.RecordDecoder
		var got []cmdproto.Record
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
		if _, _, err := decoder.Decode([]byte{1}); !errors.Is(err, cmdproto.ErrAfterCompletion) {
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
		{"unknown kind", []byte{5, 0, 0, 0, 0}, cmdproto.ErrUnknownRecord},
		{"oversized header", []byte{1, 255, 255, 255, 255}, cmdproto.ErrTooLarge},
		{"pull payload", []byte{4, 0, 0, 0, 1, 1}, cmdproto.ErrInvalidRecord},
	} {
		t.Run(tc.name, func(t *testing.T) {
			var d cmdproto.RecordDecoder
			if _, _, err := d.Decode(tc.frame); !errors.Is(err, tc.want) {
				t.Fatalf("got %v; want %v", err, tc.want)
			}
		})
	}
	complete, err := cmdproto.EncodeRecord(cmdproto.Completed{Completion: cmdproto.Completion{ExitCode: 0}})
	if err != nil {
		t.Fatal(err)
	}
	for end := 0; end < len(complete); end++ {
		var d cmdproto.RecordDecoder
		if _, _, err := d.Decode(complete[:end]); err != nil {
			t.Fatal(err)
		}
		if err := d.Finish(); !errors.Is(err, cmdproto.ErrIncomplete) {
			t.Fatalf("prefix %d: %v", end, err)
		}
	}
	for _, payload := range []string{`{"exitCode":-1}`, `{"exitCode":256}`, `{"exitCode":0,"error":null}`} {
		frame := make([]byte, 5+len(payload))
		frame[0] = 3
		binary.BigEndian.PutUint32(frame[1:], uint32(len(payload)))
		copy(frame[5:], payload)
		var d cmdproto.RecordDecoder
		if _, _, err := d.Decode(frame); err == nil {
			t.Fatalf("accepted invalid completion %s", payload)
		}
		if err := d.Finish(); !errors.Is(err, cmdproto.ErrIncomplete) {
			t.Fatalf("invalid completion %s ended stream", payload)
		}
	}
}

func TestBoundedFrames(t *testing.T) {
	data := bytes.Repeat([]byte{42}, cmdproto.MaxRecordBytes)
	input, err := cmdproto.EncodeInput(data)
	if err != nil || !bytes.Equal(input[4:], data) || binary.BigEndian.Uint32(input[:4]) != uint32(len(data)) {
		t.Fatalf("input framing: %v", err)
	}
	record, err := cmdproto.EncodeRecord(cmdproto.Stdout(data))
	if err != nil || record[0] != 1 || !bytes.Equal(record[5:], data) {
		t.Fatalf("record framing: %v", err)
	}
	data = append(data, 0)
	if _, err := cmdproto.EncodeInput(data); !errors.Is(err, cmdproto.ErrTooLarge) {
		t.Fatal(err)
	}
	if _, err := cmdproto.EncodeRecord(cmdproto.Stdout(data)); !errors.Is(err, cmdproto.ErrTooLarge) {
		t.Fatal(err)
	}
	metadata := cmdproto.LocalInvocation{
		Operation:    "echo",
		InvocationID: "1",
		Args:         []byte(`{}`),
		Cwd:          "/tmp",
		Env:          map[string]string{},
	}
	framed, err := cmdproto.EncodeMetadata(metadata)
	if err != nil {
		t.Fatal(err)
	}
	if int(binary.BigEndian.Uint32(framed[:4])) != len(framed)-4 {
		t.Fatal("incorrect metadata length")
	}
	if _, err := cmdproto.DecodeLocalInvocation(framed[4:]); err != nil {
		t.Fatal(err)
	}
	metadata.Operation = string(bytes.Repeat([]byte{'x'}, cmdproto.MaxMetadataBytes))
	if _, err := cmdproto.EncodeMetadata(metadata); !errors.Is(err, cmdproto.ErrTooLarge) {
		t.Fatal(err)
	}
}

// unrelatedMetadata models another package's value that happens to validate.
type unrelatedMetadata struct{}

func (unrelatedMetadata) Validate() error {
	return nil
}

func TestMetadataAdmission(t *testing.T) {
	if reflect.TypeOf(cmdproto.EncodeMetadata).In(0) != reflect.TypeFor[cmdproto.Metadata]() {
		t.Fatal("EncodeMetadata must take the sealed Metadata interface")
	}
	for _, tc := range []struct {
		name    string
		value   any
		allowed bool
	}{
		{"native", cmdproto.Invocation{}, true},
		{"local", cmdproto.LocalInvocation{}, true},
		{"native pointer", &cmdproto.Invocation{}, true},
		{"local pointer", &cmdproto.LocalInvocation{}, true},
		{"foreign validator", unrelatedMetadata{}, false},
		{"stream open", cmdproto.StreamOpen{}, false},
		{"conversation release", &cmdproto.ConversationRelease{}, false},
		{"conversation query", &cmdproto.ConversationQuery{}, false},
		{"completion", cmdproto.Completion{}, false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			_, ok := tc.value.(cmdproto.Metadata)
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
		{"stream open", cmdproto.StreamOpen{}.Encode, `{}`},
		{"release", func() ([]byte, error) {
			return cmdproto.EncodeConversationRequest(&cmdproto.ConversationRelease{Conversation: "conv_1"})
		}, `{"operation":"release","conversation":"conv_1"}`},
		{
			"status",
			func() ([]byte, error) {
				return cmdproto.EncodeConversationRequest(&cmdproto.ConversationQuery{})
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
	if _, err := cmdproto.EncodeConversationRequest(
		&cmdproto.ConversationRelease{Conversation: "../invalid"},
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
			return cmdproto.EncodeRecord(cmdproto.Completed{
				Completion: cmdproto.Completion{
					ExitCode: 1,
					Error: &cmdproto.CommandError{
						Code:    "failed",
						Message: text,
					},
				},
			})
		}, 5, `{"exitCode":1,"error":{"code":"failed","message":"` + text + `"}}`},
		{"metadata", func() ([]byte, error) {
			return cmdproto.EncodeMetadata(cmdproto.LocalInvocation{
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
