package transcript

import (
	"strconv"
	"strings"

	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/types"
)

// TextTokens estimates text as UTF-8 bytes divided by four, rounded up.
func TextTokens(text string) uint64 { return (uint64(len(text)) + 3) / 4 }

// BlockTokens estimates a block's text and media as request carries them.
func BlockTokens(block types.Block, request *RequestView) uint64 {
	text, media := blockEstimate(block, request)
	return TextTokens(text) + media
}

// Estimate anchors on the latest reported usage after compaction and adds later
// blocks. Without a valid anchor it sums from the last compaction boundary.
// Usage above the model's nonzero context window is not a valid anchor.
func Estimate(request *RequestView) uint64 {
	blocks := request.view.Blocks
	start := ReplayStart(blocks)
	if canAnchorUsage(blocks, start) {
		if tokens, ok := anchoredTokens(request, blocks); ok {
			return tokens
		}
	}
	var tokens uint64
	for _, block := range blocks[start:] {
		tokens += BlockTokens(block, request)
	}
	return tokens
}

// RequestSize is a request's weight as the vendor receives it.
type RequestSize struct {
	// Bytes counts base64 media and UTF-8 prompt and text, before vendor framing.
	Bytes uint64
	// Images counts image parts in both messages and tool results.
	Images uint64
}

// MeasureRequest measures already-replayed items together with the system prompt.
func MeasureRequest(systemPrompt string, items []provider.InferenceItem) RequestSize {
	size := RequestSize{Bytes: uint64(len(systemPrompt))}
	for _, item := range items {
		switch i := item.(type) {
		case *provider.UserMessage:
			size.addContent(i.Content)
		case *provider.UserSteer:
			size.addContent(i.Content)
		case *provider.AssistantText:
			size.Bytes += uint64(len(i.Text))
		case *provider.AssistantThinking:
			size.Bytes += uint64(len(i.Text))
			if i.Signature != nil {
				size.Bytes += uint64(len(*i.Signature))
			}
		case *provider.AssistantRedactedThinking:
			size.Bytes += uint64(len(i.Data))
		case *provider.ToolUse:
			size.Bytes += uint64(len(i.ToolUseID) + len(i.ToolName) + len(ToolInput(string(i.Input))))
		case *provider.ToolResult:
			size.Bytes += uint64(len(i.ToolUseID))
			for _, part := range i.Output {
				switch p := part.(type) {
				case *provider.TextPart:
					size.Bytes += uint64(len(p.Text))
				case *provider.ResultImage:
					size.Images++
					size.Bytes += types.B64Bytes(p.Bytes.Data).Base64Len()
				case *provider.ResultVideo:
					size.Bytes += types.B64Bytes(p.Bytes.Data).Base64Len()
				}
			}
		}
	}
	return size
}

// addContent weighs message parts as encoded media or UTF-8 text.
func (s *RequestSize) addContent(parts []provider.UserPart) {
	for _, part := range parts {
		switch p := part.(type) {
		case *provider.TextPart:
			s.Bytes += uint64(len(p.Text))
		case *provider.ImagePart:
			s.Images++
			s.addMedium(p.Medium)
		case *provider.VideoPart:
			s.addMedium(p.Medium)
		case *provider.DocumentPart:
			s.Bytes += types.B64Bytes(p.Bytes.Data).Base64Len()
		}
	}
}

// addMedium weighs the bytes or URL a request sends for one medium.
func (s *RequestSize) addMedium(medium provider.Medium) {
	switch m := medium.(type) {
	case *provider.MediaBytes:
		s.Bytes += types.B64Bytes(m.Data).Base64Len()
	case *provider.MediaURL:
		s.Bytes += uint64(len(m.URL))
	}
}

// blockEstimate renders the transcript text counted by compaction, before bounds.
func blockEstimate(block types.Block, request *RequestView) (string, uint64) {
	switch b := block.(type) {
	case *types.UserBlock:
		return contentEstimate(b.Content, request)
	case *types.SteerBlock:
		return contentEstimate(b.Content, request)
	case *types.ToolCallBlock:
		lines := []string{b.ToolName, b.Input}
		var media uint64
		for _, part := range b.Output {
			text, weight := resultEstimate(part, request)
			lines = append(lines, text)
			media += weight
		}
		return strings.Join(lines, "\n"), media
	case *types.WakeupBlock:
		return WakeupText, 0
	case *types.ContextBlock:
		return b.Text, 0
	case *types.AgentMessageBlock:
		// The message is already validated at entry, so its generated encoder cannot fail.
		data, _ := b.Message.MarshalJSON()
		return string(data), 0
	case *types.ResumeBlock:
		return ResumeText, 0
	case *types.ThinkingBlock:
		return b.Text, 0
	case *types.RedactedThinkingBlock:
		return b.Data, 0
	case *types.TextBlock:
		return b.Text, 0
	case *types.ResponseBlock:
		// Usage is validated when it enters the transcript.
		data, _ := b.Usage.MarshalJSON()
		return string(data), 0
	case *types.ErrorBlock:
		return b.Message, 0
	case *types.AbortBlock:
		return "aborted", 0
	case *types.CompactionBoundaryBlock:
		return b.Summary, 0
	case *types.CompactionMarkerBlock:
		return strconv.FormatUint(b.CompactedTokens, 10), 0
	}
	return "", 0
}

// contentEstimate counts each transcript part's text and native media weight.
func contentEstimate(content []types.UserContentBlock, request *RequestView) (string, uint64) {
	lines := make([]string, 0, len(content))
	var media uint64
	for _, part := range content {
		text, weight := contentPartEstimate(part, request)
		lines = append(lines, text)
		media += weight
	}
	return strings.Join(lines, "\n"), media
}

// contentPartEstimate uses the same media decision as replay without bounding text.
func contentPartEstimate(part types.UserContentBlock, request *RequestView) (string, uint64) {
	switch p := part.(type) {
	case *types.UserText:
		return p.Text, 0
	case *types.UserReference:
		return p.Reference, 0
	case *types.UserAttachment:
		return p.Name + " " + p.Path, 0
	case *types.UserImage:
		medium, text := request.medium("image", p.Source)
		switch m := medium.(type) {
		case *provider.MediaBytes:
			return m.MediaType, imageWeight(m.Data)
		case *provider.MediaURL:
			return m.URL, 1600
		}
		return text, 0
	case *types.UserVideo:
		medium, text := request.medium("video", p.Source)
		switch m := medium.(type) {
		case *provider.MediaBytes:
			return m.MediaType, 0
		case *provider.MediaURL:
			return m.URL, 0
		}
		return text, 0
	case *types.UserDocument:
		switch source := p.Source.(type) {
		case *types.DocumentRef:
			data, text := request.document(source)
			if data == nil {
				return text, 0
			}
			return source.FileName + " " + data.MediaType, (uint64(len(data.Data)) + 3) / 4
		}
	}
	return "", 0
}

// resultEstimate weighs tool output without applying replay's text bound.
func resultEstimate(part types.ToolResultContentBlock, request *RequestView) (string, uint64) {
	var kind string
	var source types.ToolMediaSource
	switch p := part.(type) {
	case *types.ToolText:
		return p.Text, 0
	case *types.ToolGone:
		return goneText(p), 0
	case *types.ToolImage:
		kind = "image"
		source = p.Source
	case *types.ToolVideo:
		kind = "video"
		source = p.Source
	}
	data, text := request.toolMedium(kind, source)
	if data == nil {
		return text, 0
	}
	if kind == "image" {
		return data.MediaType, imageWeight(data.Data)
	}
	return data.MediaType, 0
}

// imageWeight is compaction's minimum image cost or decoded bytes per 1,000.
func imageWeight(data []byte) uint64 { return max(1600, (uint64(len(data))+999)/1000) }

// canAnchorUsage requires a completed compaction before trusting reported usage.
func canAnchorUsage(blocks []types.Block, start int) bool {
	anchor := true
	if start < len(blocks) {
		if boundary, ok := blocks[start].(*types.CompactionBoundaryBlock); ok {
			anchor = false
			for _, block := range blocks {
				if marker, ok := block.(*types.CompactionMarkerBlock); ok && marker.BoundaryID == boundary.BlockID {
					anchor = true
					break
				}
			}
		}
	}
	return anchor
}

// anchoredTokens adds later block estimates to the latest usable reported usage.
func anchoredTokens(request *RequestView, blocks []types.Block) (uint64, bool) {
	for i := len(blocks) - 1; i >= 0; i-- {
		_, boundary := blocks[i].(*types.CompactionBoundaryBlock)
		_, marker := blocks[i].(*types.CompactionMarkerBlock)
		if boundary || marker {
			break
		}
		if response, ok := blocks[i].(*types.ResponseBlock); ok {
			u := response.Usage
			tokens := u.InputTokens + u.OutputTokens + u.CacheReadTokens + u.CacheWriteTokens
			if tokens == 0 {
				continue
			}
			if request.model.ContextWindow > 0 && tokens > uint64(request.model.ContextWindow) {
				break
			}
			for _, block := range blocks[i+1:] {
				tokens += BlockTokens(block, request)
			}
			return tokens, true
		}
	}
	return 0, false
}
