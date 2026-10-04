package transcript_test

import (
	"reflect"
	"testing"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/agent/transcript"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/types"
)

func TestReplayMediaUsesStableTextOrHeldBytes(t *testing.T) {
	data := types.B64Bytes{1, 2, 3}
	blob := types.BlobRefOf(data)
	source := &types.MediaSourceRef{Ref: blob, MediaType: "video/mp4"}
	document := &types.DocumentRef{Ref: blob, MediaType: "application/pdf; q=1", FileName: "a.pdf"}
	user := userBlock("u", "").(*types.UserBlock)
	user.Preamble = nil
	user.Content = []types.UserContentBlock{&types.UserVideo{Source: source}, &types.UserDocument{Source: document}}
	call := &types.ToolCallBlock{
		Status:    "error",
		ToolUseID: "call",
		ToolName:  "read",
		Input:     "{}",
		Output: []types.ToolResultContentBlock{
			&types.ToolVideo{Source: &types.ToolMediaRef{Ref: blob, MediaType: "video/mp4"}},
			&types.ToolGone{Kind: "image", MediaType: "image/png", Cause: &types.NotStored{Error: "no space"}},
			&types.ToolGone{
				Kind:      "video",
				MediaType: "video/mp4",
				Cause:     &types.Retired{At: "2026-10-01T12:00:00.000Z"},
			},
		},
	}
	blocks := []types.Block{user, call}
	model := storetest.ModelReading(
		"stub",
		"reads",
		[]types.FileExtension{types.FileExtensionMP4, types.FileExtensionPDF},
	).Model
	var held store.HeldMedia
	held.Hold(blob, data)
	missing, err := store.ReadMedia(t.Context(), storetest.NewMemoryBlobs(), []types.BlobRef{blob})
	if err != nil {
		t.Fatal(err)
	}
	for _, scenario := range []struct {
		name       string
		model      types.Model
		held       store.HeldMedia
		limit      *uint64
		video, doc string
	}{
		{"native at half limit", model, held, new(uint64(8)), "", ""},
		{
			"too big",
			model,
			held,
			new(uint64(7)),
			"[video:video/mp4, not sent: too large for the model's requests]",
			"[document:a.pdf, not sent: too large for the model's requests]",
		},
		{
			"unreadable",
			storetest.TestModel().Model,
			held,
			nil,
			"[video:video/mp4, not sent: the model does not accept it]",
			"[document:a.pdf, not sent: the model does not accept it]",
		},
		{"missing first", storetest.TestModel().Model, missing, nil, "[missing video]", "[missing document]"},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			request := requestView(
				t,
				blocks,
				scenario.model,
				scenario.held,
				provider.RequestLimits{BodyBytes: scenario.limit},
			)
			replay := transcript.Replay(request)
			message := replay.Items[0].(*provider.UserMessage)
			result := replay.Items[2].(*provider.ToolResult)
			if !result.IsError {
				t.Fatal("tool error lost")
			}
			if scenario.video == "" {
				if !reflect.DeepEqual(
					message.Content[0].(*provider.VideoPart).Medium.(*provider.MediaBytes).Data,
					[]byte(data),
				) {
					t.Fatal("video changed")
				}
				if message.Content[1].(*provider.DocumentPart).FileName != "a.pdf" {
					t.Fatal("document name lost")
				}
				if !reflect.DeepEqual(result.Output[0].(*provider.ResultVideo).Bytes.Data, []byte(data)) {
					t.Fatal("tool video changed")
				}
				if got := transcript.BlockTokens(
					user,
					request,
				); got != transcript.TextTokens(
					"video/mp4\na.pdf application/pdf; q=1",
				)+1 {
					t.Fatal(got)
				}
			} else {
				if message.Content[0].(*provider.TextPart).Text != scenario.video ||
					message.Content[1].(*provider.TextPart).Text != scenario.doc ||
					result.Output[0].(*provider.TextPart).Text != scenario.video {
					t.Fatalf("wrong replacement: %+v %+v", message, result)
				}
			}
			if result.Output[1].(*provider.TextPart).Text != "[image not stored: no space]" ||
				result.Output[2].(*provider.TextPart).Text != "[video:video/mp4, removed on 2026-10-01: "+
					"a tool result's images and videos are kept "+
					"for 30 days]" {
				t.Fatal("gone text changed")
			}
		})
	}
}

func TestRequestSizeCountsReplayedMediaAndText(t *testing.T) {
	data := provider.MediaBytes{Data: []byte{1, 2, 3, 4}, MediaType: "image/png"}
	items := []provider.InferenceItem{
		&provider.UserMessage{
			Content: []provider.UserPart{
				&provider.TextPart{Text: "你好"},
				&provider.ImagePart{Medium: &data},
				&provider.VideoPart{Medium: &provider.MediaURL{URL: "url"}},
				&provider.DocumentPart{Bytes: data},
			},
		},
		&provider.UserSteer{Content: []provider.UserPart{&provider.ImagePart{Medium: &provider.MediaURL{URL: "url"}}}},
		&provider.AssistantText{
			Text: "a",
		},
		&provider.AssistantThinking{Text: "b", Signature: new("sig")},
		&provider.AssistantRedactedThinking{Data: "opaque"},
		&provider.ToolUse{ToolUseID: "id", ToolName: "run", Input: []byte(`{"z":1,"a":2}`)},
		&provider.ToolResult{
			ToolUseID: "id",
			Output: []provider.ResultPart{
				&provider.TextPart{Text: "ok"},
				&provider.ResultImage{Bytes: data},
				&provider.ResultVideo{Bytes: data},
			},
		},
	}
	// 3 prompt + 6 UTF-8 + 8 image + 3 URL + 8 document + 3 image URL +
	// 1 answer + 1 thinking + 3 signature + 6 opaque + 2 id + 3 name +
	// 13 input + 2 result id + 2 result text + 8 image + 8 video.
	if got := transcript.MeasureRequest("sys", items); got != (transcript.RequestSize{Bytes: 80, Images: 3}) {
		t.Fatal(got)
	}
}
