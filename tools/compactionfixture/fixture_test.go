package main

import (
	"strings"
	"testing"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/core"
)

func TestRecordedFixtureLoadsIntoAgentStore(t *testing.T) {
	f, err := loadFixture()
	if err != nil {
		t.Fatal(err)
	}
	if len(f.Blocks) != 237 || f.Generations != 4 {
		t.Fatalf("got %d blocks, %d generations; want 237, 4", len(f.Blocks), f.Generations)
	}
	memory := storetest.NewMemoryTreeStore()
	initial := initialCheckpoint(f, flash(recallWindow))
	if err := memory.CreateNode(t.Context(), store.RootRecord(root, core.SystemClock{}.Now()), initial); err != nil {
		t.Fatal(err)
	}
	checkpoint, found, err := memory.Session(root).Load(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	if !found || len(checkpoint.Transcript) != 237 {
		t.Fatalf("got found=%t, %d stored blocks; want true, 237", found, len(checkpoint.Transcript))
	}
}

func TestRecallScoringAndAnswerTail(t *testing.T) {
	for _, tc := range []struct {
		answer string
		want   int
	}{
		{"ALPHA=zebra-7, BETA=QUARTZ9, GAMMA=nimbus-3", 3},
		{"ZEBRA-7 ZEBRA7 quartz-8 nimbus3", 2},
		{"ZEBRA QUARTZ NIMBUS", 0},
	} {
		if got := recalled(tc.answer); got != tc.want {
			t.Errorf("recalled(%q) = %d, want %d", tc.answer, got, tc.want)
		}
	}
	answer := strings.Repeat("😀中", 61)
	if got, want := tail(answer), strings.Repeat("😀中", 60); got != want {
		t.Errorf("tail = %q, want %q", got, want)
	}
	if got := tail("short"); got != "short" {
		t.Errorf("tail = %q, want short", got)
	}
}
