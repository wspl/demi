package transcript_test

import (
	"fmt"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/agent/transcript"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/types"
)

// estimate measures a fixture history for a model with the given context window.
func estimate(t *testing.T, blocks []types.Block, window uint32) uint64 {
	t.Helper()
	model := storetest.TestModel().Model
	model.ContextWindow = window
	return transcript.Estimate(requestView(t, blocks, model, store.HeldMedia{}, provider.RequestLimits{}))
}

func TestLatestUsageAnchorsWithinContextWindow(t *testing.T) {
	blocks := []types.Block{
		userBlock("u1", strings.Repeat("x", 40000)),
		textBlock("t", "reply"),
		responseBlock("r1", 1234),
	}
	if got := estimate(t, blocks, 1000000); got != 1384 {
		t.Fatal(got)
	}
	blocks = append(blocks, userBlock("u2", strings.Repeat("y", 4000)))
	if got := estimate(t, blocks, 1000000); got != 2384 {
		t.Fatal(got)
	}
	blocks = append(blocks, responseBlock("r2", 2000000))
	if got := estimate(t, blocks, 1000000); got <= 10000 || got >= 20000 {
		t.Fatal(got)
	}
	if got := estimate(t, blocks, 0); got != 2000150 {
		t.Fatal(got)
	}
	if transcript.TextTokens("你好") != 2 || transcript.TextTokens("hello") != 2 {
		t.Fatal("UTF-8 estimate")
	}
}

func TestCompactionInvalidatesUsageAnchor(t *testing.T) {
	boundary := &types.CompactionBoundaryBlock{BlockID: "b", Summary: "short summary", SummaryTokens: 4}
	marker := &types.CompactionMarkerBlock{BoundaryID: "b", CompactedTokens: 2000}
	// Keep usage below the context window so a mistakenly retained anchor fails.
	blocks := []types.Block{boundary, userBlock("u", strings.Repeat("x", 8000)), responseBlock("r", 900000), marker}
	for _, history := range [][]types.Block{blocks, blocks[:3]} {
		if got := estimate(t, history, 1000000); got >= 10000 {
			t.Fatal(got)
		}
	}
}

func TestMediaEstimateMatchesRequest(t *testing.T) {
	image := types.B64Bytes(make([]byte, 3000000))
	document := types.B64Bytes(strings.Repeat("a", 40000))
	screenshot := types.B64Bytes(strings.Repeat("b", 1800000))
	var held store.HeldMedia
	for _, data := range []types.B64Bytes{image, document, screenshot} {
		held.Hold(types.BlobRefOf(data), data)
	}
	user := userBlock("u", "").(*types.UserBlock)
	user.Content = []types.UserContentBlock{
		&types.UserImage{Source: &types.MediaSourceRef{Ref: types.BlobRefOf(image), MediaType: "image/png"}},
		&types.UserImage{Source: &types.MediaURL{URL: "https://example.com/a.png"}},
		&types.UserDocument{
			Source: &types.DocumentRef{
				Ref:       types.BlobRefOf(document),
				MediaType: "application/pdf",
				FileName:  "doc.pdf",
			},
		},
	}
	call := &types.ToolCallBlock{
		ToolName: "shoot",
		Input:    "{}",
		Status:   "completed",
		Output: []types.ToolResultContentBlock{
			&types.ToolImage{Source: &types.ToolMediaRef{Ref: types.BlobRefOf(screenshot), MediaType: "image/png"}},
		},
	}
	blocks := []types.Block{user, call}
	reads := storetest.ModelReading(
		"stub",
		"reads",
		[]types.FileExtension{types.FileExtensionPNG, types.FileExtensionPDF},
	).Model
	blind := storetest.TestModel().Model
	unread := func(kind, name string) string {
		return fmt.Sprintf("[%s:%s, not sent: the model does not accept it]", kind, name)
	}
	for _, scenario := range []struct {
		name   string
		model  types.Model
		limits provider.RequestLimits
		want   [2]uint64
	}{
		{
			"native",
			reads,
			provider.RequestLimits{},
			[2]uint64{
				transcript.TextTokens("image/png\nhttps://example.com/a.png\ndoc.pdf application/pdf") +
					3000 +
					1600 +
					10000,
				transcript.TextTokens("shoot\n{}\nimage/png") +
					1800,
			},
		},
		{
			"unsupported",
			blind,
			provider.RequestLimits{},
			[2]uint64{
				transcript.TextTokens(unread("image", "image/png")+
					"\nhttps://example.com/a.png\n"+
					unread("document", "doc.pdf")) +
					1600,
				transcript.TextTokens("shoot\n{}\n" +
					unread("image", "image/png")),
			},
		},
		{
			"body limit",
			reads,
			provider.RequestLimits{BodyBytes: new(uint64(5000000))},
			[2]uint64{
				transcript.TextTokens("[image:image/png, not sent: too large for the model's "+
					"requests]\nhttps://example.com/a.png\ndoc.pdf application/pdf") +
					1600 +
					10000,
				transcript.TextTokens("shoot\n{}\nimage/png") +
					1800,
			},
		},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			request := requestView(t, blocks, scenario.model, held, scenario.limits)
			got := [2]uint64{transcript.BlockTokens(user, request), transcript.BlockTokens(call, request)}
			if got != scenario.want {
				t.Fatalf("%v != %v", got, scenario.want)
			}
		})
	}
}
