package core

func BlockIDOf(block Block) BlockID {
	switch b := block.(type) {
	case BlockUser:
		return b.ID
	case *BlockUser:
		return b.ID
	case BlockContext:
		return b.ID
	case *BlockContext:
		return b.ID
	case BlockWakeup:
		return b.ID
	case *BlockWakeup:
		return b.ID
	case BlockSteer:
		return b.ID
	case *BlockSteer:
		return b.ID
	case BlockAgentMessage:
		return b.ID
	case *BlockAgentMessage:
		return b.ID
	case BlockResume:
		return b.ID
	case *BlockResume:
		return b.ID
	case BlockAbort:
		return b.ID
	case *BlockAbort:
		return b.ID
	case BlockThinking:
		return b.ID
	case *BlockThinking:
		return b.ID
	case BlockRedactedThinking:
		return b.ID
	case *BlockRedactedThinking:
		return b.ID
	case BlockText:
		return b.ID
	case *BlockText:
		return b.ID
	case BlockToolCall:
		return b.ID
	case *BlockToolCall:
		return b.ID
	case BlockResponse:
		return b.ID
	case *BlockResponse:
		return b.ID
	case BlockError:
		return b.ID
	case *BlockError:
		return b.ID
	case BlockCompactionBoundary:
		return b.ID
	case *BlockCompactionBoundary:
		return b.ID
	case BlockCompactionMarker:
		return b.ID
	case *BlockCompactionMarker:
		return b.ID
	default:
		panic("core: invalid Block")
	}
}
func BlockCreatedAt(block Block) Timestamp {
	switch b := block.(type) {
	case BlockUser:
		return b.CreatedAt
	case *BlockUser:
		return b.CreatedAt
	case BlockContext:
		return b.CreatedAt
	case *BlockContext:
		return b.CreatedAt
	case BlockWakeup:
		return b.CreatedAt
	case *BlockWakeup:
		return b.CreatedAt
	case BlockSteer:
		return b.CreatedAt
	case *BlockSteer:
		return b.CreatedAt
	case BlockAgentMessage:
		return b.CreatedAt
	case *BlockAgentMessage:
		return b.CreatedAt
	case BlockResume:
		return b.CreatedAt
	case *BlockResume:
		return b.CreatedAt
	case BlockAbort:
		return b.CreatedAt
	case *BlockAbort:
		return b.CreatedAt
	case BlockThinking:
		return b.CreatedAt
	case *BlockThinking:
		return b.CreatedAt
	case BlockRedactedThinking:
		return b.CreatedAt
	case *BlockRedactedThinking:
		return b.CreatedAt
	case BlockText:
		return b.CreatedAt
	case *BlockText:
		return b.CreatedAt
	case BlockToolCall:
		return b.CreatedAt
	case *BlockToolCall:
		return b.CreatedAt
	case BlockResponse:
		return b.CreatedAt
	case *BlockResponse:
		return b.CreatedAt
	case BlockError:
		return b.CreatedAt
	case *BlockError:
		return b.CreatedAt
	case BlockCompactionBoundary:
		return b.CreatedAt
	case *BlockCompactionBoundary:
		return b.CreatedAt
	case BlockCompactionMarker:
		return b.CreatedAt
	case *BlockCompactionMarker:
		return b.CreatedAt
	default:
		panic("core: invalid Block")
	}
}
func IsEditable(block Block) bool {
	switch block.(type) {
	case BlockUser, *BlockUser:
		return true
	}
	return false
}
