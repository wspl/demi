package core_test

import (
	"reflect"
	"strings"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/core"
)

// Encoding tests protect persisted and cross-process values; each costs under one second.
func TestBytes(t *testing.T) {
	value := core.B64Bytes{0, 1, 2, 255}
	encoded, err := contract.EncodeJSON(value)
	if err != nil || string(encoded) != `"AAEC/w=="` {
		t.Fatalf("%s: %v", encoded, err)
	}
	decoded, err := core.DecodeB64Bytes(encoded)
	if err != nil || !reflect.DeepEqual(value, decoded) {
		t.Fatalf("%v: %v", decoded, err)
	}
	for _, raw := range []string{`"AAEC/w"`, `"AAEC_w=="`, `"***"`, `null`, `[0]`, `"Zh=="`} {
		if _, err := core.DecodeB64Bytes([]byte(raw)); err == nil {
			t.Errorf("accepted %s", raw)
		}
	}
}

func TestTimes(t *testing.T) {
	value, err := core.TimestampFromMillisecond(1790000000000)
	if err != nil || value != "2026-09-21T14:13:20.000Z" {
		t.Fatalf("%s: %v", value, err)
	}
	later, err := core.TimestampFromMillisecond(1790000000500)
	if err != nil || later != "2026-09-21T14:13:20.500Z" || later <= value {
		t.Fatalf("%s: %v", later, err)
	}
	precise := time.Unix(1790000000, 999999)
	truncated, err := core.TimestampFromTime(precise)
	if err != nil || truncated != value {
		t.Fatalf("truncated %s: %v", truncated, err)
	}
	ms, err := value.Millisecond()
	if err != nil || ms != 1790000000000 {
		t.Fatalf("%d: %v", ms, err)
	}
	for _, raw := range []string{`"2026-09-21T14:13:20Z"`, `"2026-09-21T16:13:20+02:00"`, `"2026-09-21T14:13:20.0001Z"`, `"2026-09-21"`, `"yesterday"`, `1790000000000`, `"2026-02-30T14:13:20.000Z"`} {
		if _, err := core.DecodeTimestamp([]byte(raw)); err == nil {
			t.Errorf("accepted %s", raw)
		}
	}
	synctest.Test(t, func(t *testing.T) {
		var clock core.Clock = core.SystemClock{}
		got, err := clock.Now().Millisecond()
		if err != nil || got != time.Now().UnixMilli() {
			t.Fatalf("clock=%d: %v", got, err)
		}
	})
}

func TestIdentitiesAndBlobReferences(t *testing.T) {
	id, err := core.DecodeBlockID([]byte(`"b-1"`))
	if err != nil || id != "b-1" {
		t.Fatalf("%s: %v", id, err)
	}
	if _, err := core.DecodeBlockID([]byte(`""`)); err == nil {
		t.Fatal("empty identity accepted")
	}
	if _, err := core.DecodeNodeID([]byte(`7`)); err == nil {
		t.Fatal("numeric identity accepted")
	}
	digest := "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
	if got := core.BlobRefOf([]byte("test")); string(got) != digest {
		t.Fatal(got)
	}
	for _, text := range []string{strings.ToUpper(digest), digest[1:], digest + "0", "sha-1"} {
		if _, err := core.ParseBlobRef(text); err == nil {
			t.Errorf("accepted %s", text)
		}
	}
}

func TestCompletionIDs(t *testing.T) {
	id := core.CompletionID{Child: "child:1", Round: 1790000000000}
	text := "subagent:child:1:1790000000000"
	decoded, err := core.ParseCompletionID(text)
	if err != nil || decoded != id || id.String() != text {
		t.Fatalf("%v: %v", decoded, err)
	}
	block, err := id.BlockID()
	if err != nil || string(block) != text {
		t.Fatalf("%s: %v", block, err)
	}
	for _, text := range []string{"subagent:child", "subagent::5", "subagent:child:", "subagent:child:x5", "subagent:child:+5", "agent:child:5", "subagent:child:9007199254740992"} {
		if _, err := core.ParseCompletionID(text); err == nil {
			t.Errorf("accepted %s", text)
		}
	}
}
