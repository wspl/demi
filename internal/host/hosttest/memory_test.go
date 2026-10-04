package hosttest_test

import (
	"encoding/json"
	"reflect"
	"testing"

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
	if conflict, ok := storage.Apply(&host.StorageWriteIf{
		Key:      "aa",
		Value:    nil,
		Expected: &zero,
	}).(*host.StorageConflict); !ok ||
		conflict.Revision != 3 {
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
	if got := storage.Apply(&host.StorageRead{Key: "missing"}).(*host.StorageValue); got.Revision != 4 ||
		string(got.Value) != "null" {
		t.Fatalf("missing %+v", got)
	}
}
