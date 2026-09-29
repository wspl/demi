package backendtest

import (
	"encoding/json/v2"
	"reflect"
	"sync"
	"testing"
)

// Normalize returns value as loose JSON, so that a Go literal and a decoded
// document compare equal: numbers are float64, objects maps.
func Normalize(t testing.TB, value any) any {
	t.Helper()
	encoded, err := json.Marshal(value)
	if err != nil {
		t.Fatalf("%v is not JSON: %v", value, err)
	}
	var normalized any
	if err := json.Unmarshal(encoded, &normalized); err != nil {
		t.Fatal(err)
	}
	return normalized
}

// AssertJSON fails the test unless got and want are the same JSON.
func AssertJSON(t testing.TB, got, want any) {
	t.Helper()
	got, want = Normalize(t, got), Normalize(t, want)
	if !reflect.DeepEqual(got, want) {
		gotText, _ := json.Marshal(got)
		wantText, _ := json.Marshal(want)
		t.Fatalf("got  %s\nwant %s", gotText, wantText)
	}
}

// Concurrently runs count calls of work at once, each with its index, and
// waits for all of them. A call that fails the test does so with Error, not
// Fatal: it runs on a goroutine of its own.
func Concurrently(count int, work func(index int)) {
	var wg sync.WaitGroup
	for index := range count {
		wg.Add(1)
		go func() {
			defer wg.Done()
			work(index)
		}()
	}
	wg.Wait()
}

// Marshal returns the JSON of value, or fails: a scenario's own literals are
// JSON. Object members come in key order, so that a document a scenario serves
// lists its entries the same way every time.
func Marshal(value any) []byte {
	encoded, err := json.Marshal(value, json.Deterministic(true))
	if err != nil {
		panic(err)
	}
	return encoded
}

// Decode decodes a JSON document into loose JSON, or fails the test.
func Decode(t testing.TB, data []byte) any {
	t.Helper()
	var value any
	if err := json.Unmarshal(data, &value); err != nil {
		t.Fatalf("not JSON: %v\n%s", err, data)
	}
	return value
}
