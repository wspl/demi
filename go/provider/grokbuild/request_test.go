package grokbuild_test

import (
	"encoding/json/jsontext"
	"encoding/json/v2"
	"strings"
	"testing"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
	"github.com/wspl/demi/go/provider/providertest"
)

// Cost: one loopback request; the proxy's media and replay dialect stays explicit.
func TestProxyMediaAndTranscriptPolicy(t *testing.T) {
	vendor := providertest.NewMockVendor(t, providertest.MockResponse{})
	p, _ := fixture(t, vendor, false)
	request := providertest.InferenceRequest()
	request.Thinking = core.ThinkingConfigEffort{Effort: "high"}
	request.Items = []provider.InferenceItem{
		provider.UserMessage{Content: []provider.UserPart{provider.TextPart{Text: "attachment file.pdf"}, provider.ImagePart{Medium: provider.MediaBytes{Data: core.NewB64Bytes([]byte("image")), MediaType: "image/png"}}, provider.DocumentPart{Bytes: provider.MediaBytes{Data: core.NewB64Bytes([]byte("pdf")), MediaType: "application/pdf"}, FileName: "file.pdf"}, provider.VideoPart{Medium: provider.MediaURL{URL: "https://example.test/video"}}}},
		provider.AssistantThinking{Text: "private reasoning"},
		provider.AssistantText{Text: "answer"},
		provider.ToolUse{ToolUseID: "call", ToolName: "read", Input: jsontext.Value(`{"path":"file"}`)},
		provider.ToolResult{ToolUseID: "call", Output: []provider.ResultPart{provider.TextPart{Text: "contents"}}},
	}
	providertest.Run(t, runtime(t, p), request)
	raw := vendor.Requests()[0].Body
	var body struct {
		Messages  []map[string]jsontext.Value `json:"messages"`
		Reasoning string                      `json:"reasoning_effort"`
	}
	if err := json.Unmarshal(raw, &body); err != nil {
		t.Fatal(err)
	}
	if body.Reasoning != "high" || len(body.Messages) != 3 || string(body.Messages[1]["content"]) != `"answer"` || body.Messages[1]["tool_calls"] == nil || body.Messages[1]["reasoning_content"] != nil {
		t.Fatalf("transcript %s", raw)
	}
	if !strings.Contains(string(body.Messages[0]["content"]), "data:image/png;base64,aW1hZ2U=") || !strings.Contains(string(raw), "[video:https://example.test/video]") || strings.Contains(string(raw), "application/pdf") || strings.Contains(string(raw), "private reasoning") {
		t.Fatalf("media policy %s", raw)
	}
}
