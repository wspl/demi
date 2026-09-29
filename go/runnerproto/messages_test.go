package runnerproto_test

import (
	"bytes"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"errors"
	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/internal/wire"
	"github.com/wspl/demi/go/runnerproto"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// The complete Rust corpus replays in process; expected cost is under one second.
func TestMessageCorpus(t *testing.T) {
	for _, direction := range []string{"backend-to-runner", "runner-to-backend"} {
		files, err := filepath.Glob(filepath.Join("testdata", direction, "*.msgpack"))
		if err != nil || len(files) == 0 {
			t.Fatalf("corpus: %v", err)
		}
		for _, file := range files {
			t.Run(direction+"/"+filepath.Base(file), func(t *testing.T) {
				raw, err := os.ReadFile(file)
				if err != nil {
					t.Fatal(err)
				}
				var encoded []byte
				if direction == "backend-to-runner" {
					value, err := runnerproto.DecodeInboundMsgpack(raw)
					if err != nil {
						t.Fatal(err)
					}
					encoded, err = runnerproto.EncodeInboundMsgpack(value)
					if err != nil {
						t.Fatal(err)
					}
				} else {
					value, err := runnerproto.DecodeOutboundMsgpack(raw)
					if err != nil {
						t.Fatal(err)
					}
					encoded, err = runnerproto.EncodeOutboundMsgpack(value)
					if err != nil {
						t.Fatal(err)
					}
				}
				if !bytes.Equal(raw, encoded) {
					t.Fatalf("different bytes\nRust %x\nGo   %x", raw, encoded)
				}
				exportCorpus(t, filepath.Join(direction, filepath.Base(file)), encoded)
			})
		}
	}
}

// Entry refusal scenarios cover structure and domain constraints in under one second.
func TestMessageRefusals(t *testing.T) {
	inbound := []string{
		`{"type":"job_stdin","jobId":"j","bytes":[1,2]}`,
		`{"type":"ping","extra":true}`,
		`{"type":"fs_stat","id":"f","path":"/","cwd":null}`,
		`{"type":"hello_error","code":"unknown","reason":"no"}`,
		`{"type":"rpc_exit","callId":"c","exitCode":1.5}`,
		`{"type":"net_open","streamId":"n","host":"h","port":0,"input":{"id":"i","url":"/i"},"output":{"id":"o","url":"/o"}}`,
		`{"type":"artifact_location","id":"a","location":{"url":"https://example.test","expiresAt":0,"extra":true}}`,
		`{"type":"job_start","jobId":"j","context":{"conversation":"c","caller":{"kind":"user","number":1},"locale":{"timeZone":"UTC","languages":["en"]}},"script":"true","cwd":"/","env":{}}`,
		`{"type":"log_read","id":"l","limit":0}`,
		`{"type":"log_read","id":"l","limit":1001}`,
		`{"type":"numbers_reserved","id":"n","first":0}`,
	}
	outbound := []string{
		`{"type":"fs_ok","id":"f","result":true,"op":"exists"}`,
		`{"type":"fs_ok","id":"f","op":"unlink","result":null}`,
		`{"type":"fs_ok","id":"f","op":"mkdir","result":true}`,
		`{"type":"fs_ok","id":"f","op":"exists"}`,
		`{"type":"fs_ok","id":"f","op":"exists","result":true,"extra":1}`,
		`{"type":"git_ok","id":"g","op":"show","result":{}}`,
		`{"type":"spawn_exit","spawnId":"s"}`,
	}
	for _, set := range []struct {
		messages []string
		inbound  bool
	}{{inbound, true}, {outbound, false}} {
		for _, raw := range set.messages {
			data, err := wire.JSONMsgpack(jsontext.Value(raw))
			if err != nil {
				t.Fatalf("%s: %v", raw, err)
			}
			if set.inbound {
				_, err = runnerproto.DecodeInboundMsgpack(data)
			} else {
				_, err = runnerproto.DecodeOutboundMsgpack(data)
			}
			if err == nil {
				t.Fatalf("accepted %s", raw)
			}
		}
	}
	raw, err := os.ReadFile("testdata/backend-to-runner/job_start.msgpack")
	if err != nil {
		t.Fatal(err)
	}
	if _, err = runnerproto.DecodeInboundMsgpack(append(raw, 0)); err == nil {
		t.Fatal("accepted trailing data")
	}
	for _, name := range []string{"..", "../jobs", "a/b", "", strings.Repeat("a", 65)} {
		if _, err := runnerproto.EncodeInboundMsgpack(runnerproto.InboundConversationRelease{ID: "r", ConversationID: name}); err == nil {
			t.Fatalf("accepted conversation %q", name)
		}
	}
	for _, name := range []string{"provider_entry-1", strings.Repeat("a", 64)} {
		if _, err := runnerproto.EncodeInboundMsgpack(runnerproto.InboundConversationRelease{ID: "r", ConversationID: name}); err != nil {
			t.Fatal(err)
		}
	}
	for _, scenario := range []struct {
		n        uint32
		sequence commandservice.Sequence
		valid    bool
	}{{1, "tab", true}, {16, "tab", true}, {0, "tab", false}, {17, "tab", false}, {1, "command", false}} {
		_, err := runnerproto.EncodeOutboundMsgpack(runnerproto.OutboundNumbersReserve{ID: "n", ConversationID: "c", Sequence: scenario.sequence, Count: scenario.n})
		if (err == nil) != scenario.valid {
			t.Fatalf("numbers %v: %v", scenario, err)
		}
	}
	for _, status := range []string{"M ", " M", "??", "UU", "R ", "XY", "M", "  ", "???"} {
		valid := len(status) == 2 && status != "XY" && status != "  "
		_, err := runnerproto.EncodeOutboundMsgpack(runnerproto.OutboundGitOk{ID: "g", Result: runnerproto.GitResultChanges{Value: runnerproto.GitChanges{Files: []runnerproto.GitChange{{Status: status, Kind: runnerproto.ChangeKindModified}}}}})
		if (err == nil) != valid {
			t.Fatalf("status %q: %v", status, err)
		}
	}
	if _, err := runnerproto.EncodeOutboundMsgpack(runnerproto.OutboundLogLines{Lines: []runnerproto.LogLine{{At: 1, Source: ""}}}); err == nil {
		t.Fatal("accepted empty log source")
	}
}

func TestBootNormalizationAndReplyLimit(t *testing.T) {
	for _, address := range []string{"HTTP://EXAMPLE.COM:80", "HTTPS://EXAMPLE.COM:443", "WS://EXAMPLE.COM:80", "WSS://EXAMPLE.COM:443"} {
		boot := runnerproto.ManagedBoot{BackendURL: runnerproto.BackendURL(address), DeviceToken: "fake-token"}
		raw, err := boot.MarshalMsgpack()
		if err != nil {
			t.Fatal(err)
		}
		decoded, err := runnerproto.DecodeManagedBootMsgpack(raw)
		if err != nil {
			t.Fatal(err)
		}
		scheme := strings.ToLower(strings.Split(address, ":")[0])
		if decoded.BackendURL != runnerproto.BackendURL(scheme+"://example.com/") {
			t.Fatalf("URL not normalized: %s", decoded.BackendURL)
		}
	}
	called := false
	refuse := func(reason string) ([]byte, error) {
		called = true
		if reason != "the reply is 4194305 bytes, over the 4194304-byte message limit" {
			t.Fatal(reason)
		}
		return []byte("refused"), nil
	}
	if _, err := runnerproto.WithinLimit(make([]byte, 4194304), refuse); err != nil || called {
		t.Fatal("refused bounded frame")
	}
	data, err := runnerproto.WithinLimit(make([]byte, 4194305), refuse)
	if err != nil || !called || string(data) != "refused" {
		t.Fatal("oversized reply did not fail its request")
	}
}

func TestCorpusDecodesDomainValues(t *testing.T) {
	read := func(name string) runnerproto.Inbound {
		t.Helper()
		data, err := os.ReadFile("testdata/backend-to-runner/" + name + ".msgpack")
		if err != nil {
			t.Fatal(err)
		}
		value, err := runnerproto.DecodeInboundMsgpack(data)
		if err != nil {
			t.Fatal(err)
		}
		return value
	}
	stdin := read("spawn_stdin").(runnerproto.InboundSpawnStdin)
	if stdin.SpawnID != "spawn-1" || !bytes.Equal(stdin.Bytes, []byte{0, 255, 13, 10}) {
		t.Fatal("lost binary value")
	}
	times := read("fs_utimes").(runnerproto.InboundFSUtimes)
	if times.Atime != -123456789 || times.Mtime != 1790146800123 {
		t.Fatal("lost millisecond precision")
	}
	env := read("spawn").(runnerproto.InboundSpawn).Env
	if env == nil || (*env)["ANTHROPIC_API_KEY"] != nil || (*env)["PATH"] == nil || *(*env)["PATH"] != "/usr/bin:/bin" {
		t.Fatal("lost null environment entry")
	}
	context := read("job_start").(runnerproto.InboundJobStart).Context
	if context.Conversation != "conversation-1" || context.Locale.TimeZone != "Asia/Shanghai" {
		t.Fatal("lost foreign context")
	}
	location := read("artifact_location.url").(runnerproto.InboundArtifactLocation)
	raw, err := json.Marshal(location)
	if err != nil {
		t.Fatal(err)
	}
	var decoded runnerproto.InboundArtifactLocation
	if err = json.Unmarshal(raw, &decoded); err != nil {
		t.Fatal(err)
	}
	a, err := location.MarshalMsgpack()
	if err != nil {
		t.Fatal(err)
	}
	b, err := decoded.MarshalMsgpack()
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(a, b) {
		t.Fatal("foreign union JSON and MessagePack disagree")
	}
	var numbers runnerproto.OutboundNumbersReserve
	if err = json.Unmarshal([]byte(`{"type":"numbers_reserve","id":"n","conversationId":"c","sequence":"unknown","count":1}`), &numbers); err == nil {
		t.Fatal("foreign enum JSON accepted unknown sequence")
	}
	if id, ok := runnerproto.FSRequestID(times); !ok || id != times.ID {
		t.Fatal("lost request correlation")
	}
	if _, ok := runnerproto.FSRequestID(runnerproto.InboundPing{}); ok {
		t.Fatal("ping became FS request")
	}
}

func TestReplyJSONUsesTheSameResultKinds(t *testing.T) {
	for _, op := range []string{"exists", "readlink", "readdir"} {
		var reply runnerproto.OutboundFSOk
		data := []byte(`{"type":"fs_ok","id":"f","op":"` + op + `","result":null}`)
		if err := json.Unmarshal(data, &reply); err == nil {
			t.Fatalf("accepted null for %s", op)
		}
	}
}

// These boundary checks are in-memory and have no timers or external resources.
func TestRunnerPlatformRefusesMacOS(t *testing.T) {
	raw, err := os.ReadFile("testdata/runner-to-backend/hello.msgpack")
	if err != nil {
		t.Fatal(err)
	}
	message, err := runnerproto.DecodeOutboundMsgpack(raw)
	if err != nil {
		t.Fatal(err)
	}
	hello := message.(runnerproto.OutboundHello)
	hello.Runner.Platform = "macos"
	raw, err = json.Marshal(hello)
	if err != nil {
		t.Fatal(err)
	}
	packed, err := wire.JSONMsgpack(raw)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := runnerproto.DecodeOutboundMsgpack(packed); err == nil || !strings.Contains(err.Error(), "runner.platform") {
		t.Fatalf("accepted invalid platform on decoding: %v", err)
	}
	if _, err := runnerproto.EncodeOutboundMsgpack(hello); err == nil {
		t.Fatal("encoded macos")
	}
	if _, err := runnerproto.ParseRunnerPlatform("macos"); err == nil {
		t.Fatal("parsed macos")
	}
}

func TestReplyOperationErrorsNameTheTopLevelMember(t *testing.T) {
	for _, kind := range []string{"fs_ok", "git_ok"} {
		raw := []byte(`{"type":"` + kind + `","id":"r","op":"unknown","result":null}`)
		for _, packed := range []bool{false, true} {
			var err error
			if packed {
				data, packErr := wire.JSONMsgpack(raw)
				if packErr != nil {
					t.Fatal(packErr)
				}
				_, err = runnerproto.DecodeOutboundMsgpack(data)
			} else if kind == "fs_ok" {
				var reply runnerproto.OutboundFSOk
				err = json.Unmarshal(raw, &reply)
			} else {
				var reply runnerproto.OutboundGitOk
				err = json.Unmarshal(raw, &reply)
			}
			var invalid *wire.InvalidError
			if !errors.As(err, &invalid) || invalid.Path != "op" {
				t.Fatalf("%s packed=%v: %v", kind, packed, err)
			}
		}
	}
}
