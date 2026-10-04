package main

import (
	"testing"

	"github.com/vmihailenco/msgpack/v5"
	"github.com/wspl/demi/tools/contractgen/testdata/runner"
)

// Generated runner messages refuse malformed MessagePack. Local values only;
// budget one second, no processes.
func TestRunnerRefusals(t *testing.T) {
	cases := map[string]map[string]any{
		"unknown tag":          {"type": "future"},
		"missing field":        {"type": "spawn_stdin", "spawnId": "s"},
		"missing nullable":     {"type": "volume_grown", "id": "x", "volume": "home", "bytes": 1},
		"optional null string": {"type": "spawn", "spawnId": "s", "command": "echo", "cwd": nil},
		"optional null":        {"type": "spawn_kill", "spawnId": "s", "signal": nil},
		"unknown field":        {"type": "ping", "extra": true},
		"unknown enum":         {"type": "hello_error", "code": "future", "reason": "x"},
		"string bytes":         {"type": "spawn_stdin", "spawnId": "s", "bytes": "abc"},
		"array bytes":          {"type": "spawn_stdin", "spawnId": "s", "bytes": []int{1, 2}},
		"overflow":             {"type": "rpc_exit", "callId": "c", "exitCode": 256},
		"negative":             {"type": "rpc_exit", "callId": "c", "exitCode": -1},
		"float integer":        {"type": "rpc_exit", "callId": "c", "exitCode": 1.5},
		"zero volume":          {"type": "volume_grown", "id": "x", "volume": "home", "bytes": 0, "error": nil},
	}
	for name, value := range cases {
		t.Run(name, func(t *testing.T) {
			data, err := msgpack.Marshal(value)
			if err != nil {
				t.Fatal(err)
			}
			if _, err := runner.DecodeMessageMsgpack(data); err == nil {
				t.Fatal("accepted invalid message")
			}
		})
	}
	for name, data := range map[string][]byte{
		"duplicate tag": append([]byte{0x82}, []byte("\xa4type\xa4ping\xa4type\xa4ping")...),
		"trailing":      append([]byte{0x81}, []byte("\xa4type\xa4ping\xc0")...),
	} {
		t.Run(name, func(t *testing.T) {
			if _, err := runner.DecodeMessageMsgpack(data); err == nil {
				t.Fatal("accepted invalid message")
			}
		})
	}
}

func TestRunnerLargeInteger(t *testing.T) {
	original := &runner.VolumeGrown{ID: "x", Volume: "home", Bytes: 9007199254740993}
	data, err := runner.EncodeMessageMsgpack(original)
	if err != nil {
		t.Fatal(err)
	}
	got, err := runner.DecodeMessageMsgpack(data)
	if err != nil {
		t.Fatal(err)
	}
	if got.(*runner.VolumeGrown).Bytes != original.Bytes {
		t.Fatal("lost integer precision")
	}
}
