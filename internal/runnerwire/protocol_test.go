package runnerwire_test

import (
	"bytes"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/runnerwire"
)

// The corpus and refusal tests operate only on bytes and finish within one second.
func TestDirectionalCorpus(t *testing.T) {
	for _, suite := range []struct {
		dir    string
		count  int
		decode func([]byte) (any, error)
	}{
		{"backend-to-runner", 59, func(b []byte) (any, error) { return runnerwire.DecodeInbound(b) }},
		{"runner-to-backend", 56, func(b []byte) (any, error) { return runnerwire.DecodeOutbound(b) }},
	} {
		t.Run(suite.dir, func(t *testing.T) {
			files, err := filepath.Glob("testdata/" + suite.dir + "/*.msgpack")
			if err != nil {
				t.Fatal(err)
			}
			if len(files) != suite.count {
				t.Fatalf("frames=%d, want %d", len(files), suite.count)
			}
			for _, file := range files {
				t.Run(filepath.Base(file), func(t *testing.T) {
					data, err := os.ReadFile(file)
					if err != nil {
						t.Fatal(err)
					}
					message, err := suite.decode(data)
					if err != nil {
						t.Fatal(err)
					}
					encoded, err := runnerwire.Encode(message)
					if err != nil {
						t.Fatal(err)
					}
					if !bytes.Equal(encoded, data) {
						t.Fatalf("bytes changed\n got %x\nwant %x", encoded, data)
					}
				})
			}
		})
	}
}

func frame(t *testing.T, dir, name string) []byte {
	t.Helper()
	data, err := os.ReadFile("testdata/" + dir + "/" + name + ".msgpack")
	if err != nil {
		t.Fatal(err)
	}
	return data
}

// wireJSON creates malformed or valid wire inputs without bypassing the tested decoder.
func wireJSON(t *testing.T, value string) []byte {
	t.Helper()
	data, err := contract.EncodeMsgpack(json.RawMessage(value))
	if err != nil {
		t.Fatal(err)
	}
	return data
}

func TestBinaryTimesAndContexts(t *testing.T) {
	stdin, err := runnerwire.DecodeInbound(frame(t, "backend-to-runner", "spawn_stdin"))
	if err != nil {
		t.Fatal(err)
	}
	if message, ok := stdin.(*runnerwire.SpawnStdin); !ok || message.SpawnID != "spawn-1" ||
		!bytes.Equal(message.Bytes, []byte{0, 255, 13, 10}) {
		t.Fatalf("stdin=%#v", stdin)
	}
	times, err := runnerwire.DecodeInbound(frame(t, "backend-to-runner", "fs_utimes"))
	if err != nil {
		t.Fatal(err)
	}
	if message, ok := times.(*runnerwire.FSUtimes); !ok || message.Atime != -123456789 ||
		message.Mtime != 1790146800123 {
		t.Fatalf("times=%#v", times)
	}
	start, err := runnerwire.DecodeInbound(frame(t, "backend-to-runner", "job_start"))
	if err != nil {
		t.Fatal(err)
	}
	job, ok := start.(*runnerwire.JobStart)
	if !ok {
		t.Fatalf("job=%T", start)
	}
	agent, ok := job.Context.Caller.(*commandwire.AgentCaller)
	if !ok || agent.Number != 1 || job.Context.Conversation != "conversation-1" ||
		job.Context.Locale.TimeZone != "Asia/Shanghai" ||
		!reflect.DeepEqual(job.Context.Locale.Languages, []commandwire.LanguageTag{"zh-CN", "en"}) {
		t.Fatalf("context=%#v", job.Context)
	}
	spawn, err := runnerwire.DecodeInbound(frame(t, "backend-to-runner", "spawn"))
	if err != nil {
		t.Fatal(err)
	}
	message, ok := spawn.(*runnerwire.Spawn)
	if !ok || message.Env == nil {
		t.Fatalf("spawn=%#v", spawn)
	}
	env := *message.Env
	if token, ok := env["ANTHROPIC_API_KEY"]; !ok || token != nil {
		t.Fatal("nullable environment entry lost")
	}
	if env["PATH"] == nil || *env["PATH"] != "/usr/bin:/bin" {
		t.Fatal("PATH lost")
	}
}

func TestInboundRefusals(t *testing.T) {
	for _, value := range []string{
		`{"type":"job_stdin","jobId":"job","bytes":[1,2]}`,
		`{"type":"job_stdin","jobId":"job","bytes":"text"}`,
		`{"type":"ping","extra":true}`,
		`{"type":"fs_stat","id":"file","path":"/work","cwd":null}`,
		`{"type":"hello_error","code":"unknown","reason":"no"}`,
		`{"type":"rpc_exit","callId":"call","exitCode":1.5}`,
		`{"type":"net_open","streamId":"net","host":"h","port":0,"input":{"id":"i",` +
			`"url":"/i"},"output":{"id":"o","url":"/o"}}`,
	} {
		if _, err := runnerwire.DecodeInbound(wireJSON(t, value)); err == nil {
			t.Errorf("accepted %s", value)
		}
	}
	if _, err := runnerwire.DecodeInbound(wireJSON(t, `{"type":"fs_stat","id":"file","path":"/work"}`)); err != nil {
		t.Fatal(err)
	}
	data := append(frame(t, "backend-to-runner", "job_start"), 0)
	if _, err := runnerwire.DecodeInbound(data); err == nil {
		t.Fatal("accepted trailing data")
	}
}

func TestNestedValidation(t *testing.T) {
	for _, tc := range []struct {
		conversation, caller, hash string
		valid                      bool
	}{
		{"", `{"kind":"agent","number":1}`, "", false},
		{"conversation", `{"kind":"agent","number":1}`, "", true},
		{"conversation", `{"kind":"user","number":1}`, "", false},
		{"c", `{"kind":"user"}`, `,"manifestHash":"not a digest"`, false},
	} {
		value := fmt.Sprintf(
			`{"type":"job_start","jobId":"job","context":{"conversation":%q,"caller":%s,`+
				`"locale":{"timeZone":"UTC","languages":["en"]}},"script":"true","cwd":"/","env":{}%s}`,
			tc.conversation,
			tc.caller,
			tc.hash,
		)
		_, err := runnerwire.DecodeInbound(wireJSON(t, value))
		if (err == nil) != tc.valid {
			t.Errorf("%s: %v", value, err)
		}
	}
}

func TestConversationNames(t *testing.T) {
	for _, tc := range []struct {
		name  string
		valid bool
	}{
		{"0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a01", true},
		{"provider_entry-1", true},
		{strings.Repeat("a", 64), true},
		{"..", false},
		{"../jobs", false},
		{"a/b", false},
		{"", false},
		{strings.Repeat("a", 65), false},
	} {
		for _, value := range []string{
			fmt.Sprintf(`{"type":"conversation_release","id":"release","conversationId":%q}`, tc.name),
			fmt.Sprintf(`{"type":"job_start","jobId":"job","context":{"conversation":%q,`+
				`"caller":{"kind":"user"},"locale":{"timeZone":"UTC","languages":["en"]}},`+
				`"script":"true","cwd":"/","env":{}}`, tc.name),
		} {
			_, err := runnerwire.DecodeInbound(wireJSON(t, value))
			if (err == nil) != tc.valid {
				t.Errorf("%s: %v", value, err)
			}
		}
	}
}

func TestLogBounds(t *testing.T) {
	for _, limit := range []int{0, 1001} {
		_, err := runnerwire.DecodeInbound(wireJSON(t, fmt.Sprintf(`{"type":"log_read","id":"log","limit":%d}`, limit)))
		if err == nil {
			t.Errorf("accepted limit %d", limit)
		}
	}
	_, err := runnerwire.Encode(
		&runnerwire.LogLines{
			ID:    "log",
			Lines: []runnerwire.LogLine{{At: 1700000000123, Source: "", Text: "could not list tabs"}},
			Next:  42,
		},
	)
	if err == nil {
		t.Fatal("encoded unnamed log source")
	}
}

func TestNumberReservations(t *testing.T) {
	for _, tc := range []struct {
		count    int
		sequence string
		valid    bool
	}{{1, "tab", true}, {16, "tab", true}, {0, "tab", false}, {17, "tab", false}, {1, "command", false}} {
		_, err := runnerwire.DecodeOutbound(
			wireJSON(
				t,
				fmt.Sprintf(
					`{"type":"numbers_reserve","id":"numbers","conversationId":"c","sequence":%q,"count":%d}`,
					tc.sequence,
					tc.count,
				),
			),
		)
		if (err == nil) != tc.valid {
			t.Errorf("%+v: %v", tc, err)
		}
	}
	for _, first := range []int{0, 1} {
		_, err := runnerwire.DecodeInbound(
			wireJSON(t, fmt.Sprintf(`{"type":"numbers_reserved","id":"numbers","first":%d}`, first)),
		)
		if (err == nil) != (first == 1) {
			t.Errorf("first %d: %v", first, err)
		}
	}
}

func TestReplyOperations(t *testing.T) {
	for _, value := range []string{
		`{"type":"fs_ok","id":"fs","result":true,"op":"exists"}`,
		`{"type":"fs_ok","id":"fs","op":"unlink","result":null}`,
		`{"type":"fs_ok","id":"fs","op":"mkdir","result":true}`,
		`{"type":"fs_ok","id":"fs","op":"exists"}`,
		`{"type":"fs_ok","id":"fs","op":"exists","result":true,"extra":1}`,
		`{"type":"git_ok","id":"git","op":"show","result":{}}`,
		`{"type":"spawn_exit","spawnId":"spawn"}`,
	} {
		if _, err := runnerwire.DecodeOutbound(wireJSON(t, value)); err == nil {
			t.Errorf("accepted %s", value)
		}
	}
}

func TestGitStatusPairs(t *testing.T) {
	for _, tc := range []struct {
		status string
		valid  bool
	}{
		{
			"M ",
			true,
		},
		{
			" M",
			true,
		},
		{
			"??",
			true,
		},
		{
			"UU",
			true,
		},
		{
			"R ",
			true,
		},
		{
			"XY",
			false,
		},
		{
			"M",
			false,
		},
		{
			"  ",
			false,
		},
		{
			"???",
			false,
		},
	} {
		value := fmt.Sprintf(
			`{"type":"git_ok","id":"git","op":"changes","result":{"repository":true,"head":null,`+
				`"truncated":false,"watched":false,"files":[{"path":"a","status":%q,"kind":"modified",`+
				`"added":0,"removed":0}]}}`,
			tc.status,
		)
		_, err := runnerwire.DecodeOutbound(wireJSON(t, value))
		if (err == nil) != tc.valid {
			t.Errorf("status %q: %v", tc.status, err)
		}
	}
}

func TestRunnerPlatforms(t *testing.T) {
	data := frame(t, "runner-to-backend", "hello")
	for name, platform := range map[string]runnerwire.RunnerPlatform{
		"darwin": runnerwire.RunnerPlatformDarwin,
		"win32":  runnerwire.RunnerPlatformWin32,
		"linux":  runnerwire.RunnerPlatformLinux,
	} {
		if string(platform) != name {
			t.Fatalf("platform %q, want %q", platform, name)
		}
		message, err := runnerwire.DecodeOutbound(data)
		if err != nil {
			t.Fatal(err)
		}
		hello, ok := message.(*runnerwire.Hello)
		if !ok {
			t.Fatalf("hello=%T", message)
		}
		hello.Runner.Platform = platform
		encoded, err := runnerwire.Encode(hello)
		if err != nil {
			t.Fatal(err)
		}
		decoded, err := runnerwire.DecodeOutbound(encoded)
		if err != nil {
			t.Fatal(err)
		}
		roundtrip, ok := decoded.(*runnerwire.Hello)
		if !ok || roundtrip.Runner.Platform != platform {
			t.Fatalf("hello=%#v", decoded)
		}
		stored, err := runnerwire.ParseRunnerPlatform(string(platform))
		if err != nil || stored != platform {
			t.Fatalf("stored=%s, %v", stored, err)
		}
	}
	if _, err := runnerwire.ParseRunnerPlatform("macos"); err == nil {
		t.Fatal("accepted macos")
	}
	// Patch the recorded string without using a validating encoder.
	invalid := bytes.Replace(data, []byte("darwin"), []byte("macos!"), 1)
	if bytes.Equal(invalid, data) {
		t.Fatal("fixture platform was not darwin")
	}
	if _, err := runnerwire.DecodeOutbound(invalid); err == nil {
		t.Fatal("accepted invalid hello platform")
	}
}

func TestInstallAndServiceBounds(t *testing.T) {
	install := runnerwire.Install{
		Package: "demi.browser",
		Name:    "Chrome for Testing",
		Version: "153.0.8010.36",
		Phase:   runnerwire.InstallPhaseDownload,
		Done:    1,
		Total:   2,
	}
	if _, err := runnerwire.Encode(&runnerwire.Installs{Installs: []runnerwire.Install{install}}); err != nil {
		t.Fatal(err)
	}
	for _, mutate := range []func(*runnerwire.Install){
		func(v *runnerwire.Install) { v.Done = 3 },
		func(v *runnerwire.Install) { v.Total = 0 },
		func(v *runnerwire.Install) { v.Name = strings.Repeat("😀", 26) },
		func(v *runnerwire.Install) { v.Package = strings.Repeat("😀", 51) },
		func(v *runnerwire.Install) { v.Version = strings.Repeat("😀", 26) },
	} {
		changed := install
		mutate(&changed)
		if _, err := runnerwire.Encode(&runnerwire.Installs{Installs: []runnerwire.Install{changed}}); err == nil {
			t.Fatal("accepted invalid installation")
		}
	}
	installs := make([]runnerwire.Install, runnerwire.MaxInstalls+1)
	for i := range installs {
		installs[i] = install
	}
	if _, err := runnerwire.Encode(&runnerwire.Installs{Installs: installs}); err == nil {
		t.Fatal("accepted too many installations")
	}
	for _, size := range []int{runnerwire.ServiceStderrChars, runnerwire.ServiceStderrChars + 1} {
		_, err := runnerwire.Encode(&runnerwire.ServiceDone{StreamID: "s", Stderr: strings.Repeat("😀", size)})
		if (err == nil) != (size == runnerwire.ServiceStderrChars) {
			t.Fatalf("stderr size %d: %v", size, err)
		}
	}
}

func TestOversizedReplyFailsItsRequest(t *testing.T) {
	raw, err := runnerwire.Encode(
		&runnerwire.FSError{ID: "request-1", Message: strings.Repeat("x", runnerwire.MaxMessageBytes)},
	)
	if err != nil {
		t.Fatal(err)
	}
	reply, err := runnerwire.WithinLimit(raw, func(reason string) ([]byte, error) {
		return runnerwire.Encode(&runnerwire.FSError{ID: "request-1", Message: reason})
	})
	if err != nil {
		t.Fatal(err)
	}
	decoded, err := runnerwire.DecodeOutbound(reply)
	if err != nil {
		t.Fatal(err)
	}
	refusal, ok := decoded.(*runnerwire.FSError)
	if !ok || refusal.ID != "request-1" || !strings.Contains(refusal.Message, "message limit") {
		t.Fatalf("reply=%#v", decoded)
	}
	same, err := runnerwire.WithinLimit(
		reply,
		func(string) ([]byte, error) { t.Fatal("refused a small reply"); return nil, nil },
	)
	if err != nil || !bytes.Equal(same, reply) {
		t.Fatal("small reply changed")
	}
}

func TestRequestIDs(t *testing.T) {
	files, err := filepath.Glob("testdata/backend-to-runner/*.msgpack")
	if err != nil {
		t.Fatal(err)
	}
	for _, file := range files {
		data, err := os.ReadFile(file)
		if err != nil {
			t.Fatal(err)
		}
		message, err := runnerwire.DecodeInbound(data)
		if err != nil {
			t.Fatal(err)
		}
		name := filepath.Base(file)
		fsID, isFS := runnerwire.FSRequestID(message)
		gitID, isGit := runnerwire.GitRequestID(message)
		if isFS != strings.HasPrefix(name, "fs_") || isGit != strings.HasPrefix(name, "git_") {
			t.Errorf("%s: fs=%t git=%t", name, isFS, isGit)
		}
		if isFS && fsID == "" || isGit && gitID == "" {
			t.Errorf("%s: lost correlation id", name)
		}
	}
}

func TestOpaqueManifestDefersVerification(t *testing.T) {
	// This transport fixture deliberately carries a placeholder hash and empty
	// command group. Wire decoding preserves it; manifest verification refuses it.
	message, err := runnerwire.DecodeInbound(frame(t, "backend-to-runner", "manifest"))
	if err != nil {
		t.Fatal(err)
	}
	envelope, ok := message.(*runnerwire.ManifestMessage)
	if !ok {
		t.Fatalf("manifest=%T", message)
	}
	if _, err := runnerwire.DecodeManifest(envelope.Manifest); err == nil {
		t.Fatal("accepted empty command group and placeholder digest")
	}
}

func TestGitChangesJSON(t *testing.T) {
	data := []byte(
		`{"repository":true,"head":null,"files":[{"path":"a<&>","status":" M",` +
			`"kind":"modified","from":"old","added":2,"removed":1}],"truncated":false,` +
			`"watched":true}`,
	)
	changes, err := runnerwire.DecodeGitChanges(data)
	if err != nil {
		t.Fatal(err)
	}
	encoded, err := contract.EncodeJSON(changes)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(encoded, data) {
		t.Fatalf("GitChanges JSON differs from the wire form: %s", encoded)
	}
	for _, invalid := range [][]byte{
		bytes.Replace(data, []byte(`"head":null,`), nil, 1),
		bytes.Replace(data, []byte(`"from":"old"`), []byte(`"from":null`), 1),
		bytes.Replace(data, []byte(`"added":2`), []byte(`"added":9007199254740992`), 1),
	} {
		if _, err := runnerwire.DecodeGitChanges(invalid); err == nil {
			t.Errorf("accepted invalid JSON: %s", invalid)
		}
	}
}
