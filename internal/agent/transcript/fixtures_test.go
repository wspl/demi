package transcript_test

import (
	"testing"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/agent/transcript"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
	"go.uber.org/goleak"
)

func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }

// userBlock supplies a transcript message, including a preamble estimates omit.
func userBlock(id, text string) core.Block {
	return &core.UserBlock{
		BlockID:   core.BlockID(id),
		TurnID:    core.TurnID(id),
		Timestamp: core.UnixEpoch,
		Selection: storetest.TestModel(),
		Content:   storetest.Text(text),
		Preamble:  new("not counted"),
	}
}

// textBlock supplies an assistant answer for transcript boundary scenarios.
func textBlock(id, text string) core.Block {
	return &core.TextBlock{
		BlockID:   core.BlockID(id),
		Timestamp: core.UnixEpoch,
		Selection: storetest.TestModel(),
		Text:      text,
	}
}

// responseBlock supplies a measured provider response with nonzero cache usage.
func responseBlock(id string, input uint64) core.Block {
	return &core.ResponseBlock{
		BlockID:   core.BlockID(id),
		Timestamp: core.UnixEpoch,
		Selection: storetest.TestModel(),
		Usage:     core.TokenUsage{InputTokens: input, OutputTokens: 50, CacheReadTokens: 100},
	}
}

// requestView builds a model view and resolves any intentionally absent fixture blobs.
func requestView(
	t *testing.T,
	blocks []core.Block,
	model core.Model,
	held store.HeldMedia,
	limits provider.RequestLimits,
) *transcript.RequestView {
	t.Helper()
	view, missing := store.NewModelView(0, blocks, held)
	if len(missing) > 0 {
		t.Fatalf("fixture has unheld media: %v", missing)
	}
	return transcript.NewRequestView(view, model, limits)
}
