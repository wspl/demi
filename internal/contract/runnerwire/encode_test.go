package runnerwire

import (
	"errors"
	"testing"

	"github.com/wspl/demi/internal/contract/cmdservice"
	"github.com/wspl/demi/internal/contract/zodrt"
)

func validRunnerInfo() RunnerInfo {
	return RunnerInfo{
		Name:     "laptop",
		Platform: "linux",
		Version:  "1.0.0",
		Identity: HostIdentity{UID: 1000, GID: 1000, Hostname: "laptop", HomeDir: "/home/demi"},
	}
}

func TestEncodersRefuseInvalidValues(t *testing.T) {
	pipe := PipeRef{ID: "p", URL: "/pipes/p"}
	invalid := map[string]BackendMessage{
		"port below range":      NetOpen{StreamID: "s", Host: "example.com", Port: 0, Input: pipe, Output: pipe},
		"log limit above range": LogRead{ID: "r", Limit: LogReadLines + 1},
		"unsafe integer":        LogRead{ID: "r", Limit: 1, Since: zodrt.Some(int64(1) << 53)},
		"empty conversation":    ConversationRelease{ID: "c", ConversationID: ""},
		"missing union value":   nil,
	}
	for name, message := range invalid {
		if _, err := EncodeBackendMessageMsgpack(message); err == nil {
			t.Errorf("%s: encoded", name)
		}
		if _, err := EncodeBackendMessageJSON(message); err == nil {
			t.Errorf("%s: encoded as JSON", name)
		}
	}
	info := validRunnerInfo()
	info.NativeTarget = zodrt.Some(cmdservice.NativeTarget("sparc-sun-solaris"))
	if _, err := EncodeRunnerMessageMsgpack(Hello{Protocol: RunnerProtocolVersion, Runner: info}); err == nil {
		t.Error("an unknown native target encoded")
	}
	if _, err := EncodeRunnerMessageMsgpack(ServiceDone{StreamID: "s", ExitCode: 256}); err == nil {
		t.Error("an exit code above 255 encoded")
	}
	if _, err := EncodeManagedBootJSON(ManagedBoot{BackendURL: "ftp://backend", DeviceToken: "t"}); err == nil {
		t.Error("a managed boot with a non-HTTP backend encoded")
	}
}

func TestMessagesRoundTrip(t *testing.T) {
	message := Hello{Protocol: RunnerProtocolVersion, DeviceToken: zodrt.Some("token"), Runner: validRunnerInfo()}
	frame, err := EncodeRunnerMessageFrame(message)
	if err != nil {
		t.Fatal(err)
	}
	decoded, err := DecodeRunnerMessageFrame(frame)
	if err != nil {
		t.Fatal(err)
	}
	if got, ok := decoded.(Hello); !ok || got.DeviceToken.Value != "token" || got.Runner.Identity.HomeDir != "/home/demi" {
		t.Fatalf("decoded %#v", decoded)
	}
	request := FsStat{ID: "42", Path: "a.txt"}
	if id, ok := FsRequestID(request); !ok || id != "42" {
		t.Errorf("fs request id %q %v", id, ok)
	}
	if _, ok := GitRequestID(request); ok {
		t.Error("a file request has a working-tree request id")
	}
}

func TestFramesStayWithinTheLimit(t *testing.T) {
	_, err := EncodeBackendMessageFrame(JobStdin{JobID: "j", Bytes: make([]byte, MaxMessageBytes)})
	var tooLarge *MessageTooLargeError
	if !errors.As(err, &tooLarge) || tooLarge.Code() != "too_large" {
		t.Fatalf("an oversized frame: %v", err)
	}
	if _, err := DecodeBackendMessageFrame(make([]byte, MaxMessageBytes+1)); err == nil {
		t.Error("an oversized frame decoded")
	}
}
