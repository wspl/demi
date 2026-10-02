package hosttest_test

import (
	"context"
	"encoding/json"
	"errors"
	"reflect"
	"testing"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/host/hosttest"
)

func TestStorageRevisionsSortedKeysAndDeletion(t *testing.T) {
	var storage hosttest.MemoryStorage
	for _, key := range []string{"b", "ab", "aa"} {
		storage.Apply(&host.StorageWriteIf{Key: key, Value: json.RawMessage(`1`)})
	}
	read := storage.Apply(&host.StorageRead{Key: "aa"}).(*host.StorageValue)
	if read.Revision != 3 || string(read.Value) != "1" {
		t.Fatalf("read %+v", read)
	}
	zero := host.Revision(0)
	if conflict, ok := storage.Apply(&host.StorageWriteIf{Key: "aa", Value: nil, Expected: &zero}).(*host.StorageConflict); !ok || conflict.Revision != 3 {
		t.Fatalf("conflict %+v", conflict)
	}
	keys := storage.Apply(&host.StorageList{Prefix: "a"}).(*host.StorageKeys)
	if !reflect.DeepEqual(keys.Keys, []string{"aa", "ab"}) {
		t.Fatal(keys.Keys)
	}
	storage.Apply(&host.StorageWriteIf{Key: "aa", Value: json.RawMessage("null"), Expected: &read.Revision})
	if storage.Value("aa") != nil {
		t.Fatal("delete retained key")
	}
	if got := storage.Apply(&host.StorageRead{Key: "missing"}).(*host.StorageValue); got.Revision != 4 || string(got.Value) != "null" {
		t.Fatalf("missing %+v", got)
	}
}
func TestPagesAndConversationNumbers(t *testing.T) {
	pages := hosttest.NewPages(false)
	watching, changed := pages.Watching()
	if watching {
		t.Fatal("watching initially")
	}
	pages.Watch(true)
	select {
	case <-changed:
	default:
		t.Fatal("watcher not notified")
	}
	if watching, _ = pages.Watching(); !watching {
		t.Fatal("watcher state")
	}
	record := host.NewCommandRecord("s", "c", "tool")
	pages.Changed(record)
	record.AppendOutput("stdout", "hello")
	pages.Changed(record)
	first, err := pages.Next(t.Context())
	if err != nil || first.Chars != 0 {
		t.Fatalf("first %+v %v", first, err)
	}
	rest := pages.Drain()
	if len(rest) != 1 || rest[0].Tail != "hello" || len(pages.Drain()) != 0 {
		t.Fatalf("drain %+v", rest)
	}
	ctx, cancel := context.WithCancel(t.Context())
	cancel()
	if _, err = pages.Next(ctx); !errors.Is(err, context.Canceled) {
		t.Fatalf("cancel %v", err)
	}
	var numbers hosttest.CountingNumbers
	for _, test := range []struct {
		sequence string
		want     uint64
	}{{"command", 1}, {"shell", 1}, {"command", 2}} {
		// Sequences are internal values, supplied by the component asking for numbers.
		sequence := core.Sequence(test.sequence)
		got, err := numbers.Next(t.Context(), sequence)
		if err != nil || got != test.want {
			t.Fatalf("number %d %v", got, err)
		}
	}
}
