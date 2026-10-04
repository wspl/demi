package cmdsdktest_test

import (
	"context"
	"errors"
	"io"
	"testing"

	"github.com/wspl/demi/internal/cmdproto"
	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/cmdsdk/cmdsdktest"
	"go.uber.org/goleak"
)

func TestMain(m *testing.M) {
	goleak.VerifyTestMain(m)
}

// Local fixtures have the same per-conversation sequence and artifact outcome
// semantics as side streams, and their test cleanup ends outstanding sources.
func TestLocalSourcesOwnCleanupAndPreserveAnswers(t *testing.T) {
	var numbers *cmdsdk.Numbers
	var artifacts *cmdsdk.Artifacts
	t.Run("owner", func(t *testing.T) {
		numbers = cmdsdktest.CountingNumbers(t)
		for _, tc := range []struct {
			conversation string
			count        uint32
			first        uint64
		}{{"one", 4, 1}, {"one", 1, 5}, {"two", 1, 1}} {
			first, err := numbers.Draw(t.Context(), tc.conversation, cmdproto.TabSequence, tc.count)
			if err != nil || first != tc.first {
				t.Fatalf("draw %d: %v", first, err)
			}
		}
		artifacts = cmdsdktest.ArtifactsFrom(
			t,
			func(_ context.Context, q cmdproto.ArtifactRequest) (cmdproto.ArtifactAnswer, error) {
				if q.Installed.Name == "failed" {
					return cmdproto.ArtifactAnswer{}, errors.New("unavailable")
				}
				values := []cmdproto.InstalledArtifact{}
				return cmdproto.ArtifactAnswer{Installed: &values}, nil
			},
		)
		values, err := artifacts.Installed(t.Context(), "empty")
		if err != nil || len(values) != 0 {
			t.Fatalf("installed: %v %v", values, err)
		}
		if _, err := artifacts.Installed(t.Context(), "failed"); err == nil {
			t.Fatal("lost artifact failure")
		}
	})
	if _, err := numbers.Draw(t.Context(), "one", cmdproto.TabSequence, 1); !errors.Is(err, io.EOF) {
		t.Fatalf("numbers after cleanup: %v", err)
	}
	if _, err := artifacts.Installed(t.Context(), "empty"); !errors.Is(err, io.EOF) {
		t.Fatalf("artifacts after cleanup: %v", err)
	}
}
