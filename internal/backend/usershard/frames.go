package usershard

import (
	"context"
	"errors"
	"fmt"
	"log/slog"

	"github.com/wspl/demi/internal/agent/server"
	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/hostaccess"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/webapi"
)

type frameRefusalError struct {
	code    webapi.ErrorCode
	message string
}

// Error returns the frame refusal text shown to the client.
func (r *frameRefusalError) Error() string { return r.message }

func (s *Shard) prepareFrame(
	ctx context.Context,
	id webapi.ConversationID,
	frame framewire.ClientFrame,
) error {
	record, err := s.Control().Conversation(ctx, id)
	if err != nil {
		return err
	}
	if record == nil || record.Owner != s.user {
		return &frameRefusalError{
			webapi.ErrorCodeConversationNotFound,
			"No such conversation",
		}
	}
	if record.Archived && frame.Kind() != framewire.ClientFrameKindClose {
		return &frameRefusalError{
			webapi.ErrorCodeConversationArchived,
			"Restore the conversation before writing to it",
		}
	}
	switch frame := frame.(type) {
	case *framewire.OpenFrame:
		return s.prepareOpen(ctx, *record)
	case *framewire.SendFrame:
		return s.prepareSend(ctx, *record, frame)
	case *framewire.SteerFrame, *framewire.EditAndSendFrame:
		if _, err := s.Control().CountUserMessage(ctx, id); err != nil {
			return err
		}
		s.Mark(pagesync.Part{Kind: pagesync.Conversation, ConversationID: id})
	case *framewire.AbortFrame,
		*framewire.AbortSubagentFrame,
		*framewire.AbortSubagentsFrame,
		*framewire.CancelPendingSteerFrame,
		*framewire.ClearMessageQueueFrame,
		*framewire.CloseFrame,
		*framewire.CompactFrame,
		*framewire.DequeueMessageFrame,
		*framewire.ResumeFrame,
		*framewire.RetryFrame,
		*framewire.SendQueuedMessageFrame,
		*framewire.ShellAbortFrame,
		*framewire.ShellWriteFrame,
		*framewire.SteerQueuedMessageFrame,
		*framewire.SyncTranscriptFrame:
	}
	return nil
}

func (s *Shard) restoreTree(ctx context.Context, id webapi.ConversationID) error {
	record, err := hostaccess.OwnedConversation(ctx, s, id)
	if err != nil {
		return err
	}
	target, err := hostaccess.ResolveTarget(ctx, s, record)
	if err != nil {
		return err
	}
	slot := s.conversations.Slot(id)
	files, err := slot.FileGate().Enter(ctx, gates.Demand)
	if err != nil {
		return err
	}
	defer files.Release()
	settings, err := slot.Settings().Acquire(ctx)
	if err != nil {
		return err
	}
	defer settings.Release()
	if err := s.prepareFrame(ctx, id, &framewire.OpenFrame{}); err != nil {
		return err
	}
	return s.agent.Restore(ctx, hostaccess.RootOf(id), database.ExecutionPath(target))
}

func refusedFrame(frame framewire.ClientFrame, code webapi.ErrorCode, message string) framewire.ServerFrame {
	if edit, ok := frame.(*framewire.EditAndSendFrame); ok {
		return &framewire.EditResultFrame{
			OperationID: edit.Request.OperationID,
			Outcome:     &framewire.RejectedEdit{Reason: message},
		}
	}
	name := string(code)
	return &framewire.ErrorFrame{Message: message, Code: &name}
}

func carriesUploads(frame framewire.ClientFrame) bool {
	var content []framewire.ClientContent
	switch frame := frame.(type) {
	case *framewire.SendFrame:
		content = frame.Content
	case *framewire.SteerFrame:
		content = frame.Content
	case *framewire.EditAndSendFrame:
		content = frame.Request.Content
	case *framewire.AbortFrame,
		*framewire.AbortSubagentFrame,
		*framewire.AbortSubagentsFrame,
		*framewire.CancelPendingSteerFrame,
		*framewire.ClearMessageQueueFrame,
		*framewire.CloseFrame,
		*framewire.CompactFrame,
		*framewire.DequeueMessageFrame,
		*framewire.OpenFrame,
		*framewire.ResumeFrame,
		*framewire.RetryFrame,
		*framewire.SendQueuedMessageFrame,
		*framewire.ShellAbortFrame,
		*framewire.ShellWriteFrame,
		*framewire.SteerQueuedMessageFrame,
		*framewire.SyncTranscriptFrame:
	}
	for _, part := range content {
		if _, ok := part.(*framewire.UploadContent); ok {
			return true
		}
	}
	return false
}

func (s *Shard) handleMessage(
	ctx context.Context,
	id webapi.ConversationID,
	connection *server.Connection[*remotehost.Host],
	files *conversationFiles,
	text []byte,
) (framewire.ServerFrame, error) {
	frame, err := framewire.DecodeClientFrame(text)
	if errors.Is(err, framewire.ErrNotJSON) {
		return nil, err
	}
	if err != nil {
		return refusedFrame(nil, webapi.ErrorCodeInvalidFrame, fmt.Sprintf("Invalid client frame: %v", err)), nil
	}
	if carriesUploads(frame) {
		admitted, err := hostaccess.AdmitHost(ctx, s, id, nil)
		if err != nil {
			code := uploadFailureCode(err)
			return refusedFrame(frame, code, err.Error()), nil
		}
		defer admitted.Release()
		files.host = &admitted.Host
		defer func() {
			files.host = nil
		}()
	} else {
		admitted, err := s.conversations.Slot(id).FileGate().Enter(ctx, gates.Demand)
		if err != nil {
			return nil, err
		}
		defer admitted.Release()
	}
	if frame.Kind() == framewire.ClientFrameKindOpen {
		settings, err := s.conversations.Slot(id).Settings().Acquire(ctx)
		if err != nil {
			return nil, err
		}
		defer settings.Release()
	}
	if err := s.prepareFrame(ctx, id, frame); err != nil {
		code := webapi.ErrorCodeFrameDeliveryFailed
		var refused *frameRefusalError
		if errors.As(err, &refused) {
			code = refused.code
		} else {
			slog.ErrorContext(ctx, "a frame was not prepared", "conversation", id, "error", err)
		}
		return refusedFrame(frame, code, err.Error()), nil
	}
	connection.Handle(ctx, frame)
	return nil, nil
}

type conversationFiles struct {
	shard        *Shard
	conversation webapi.ConversationID
	host         *hostaccess.ConversationHost
}

// Resolve admits uploaded and remote files as conversation content.
func (f *conversationFiles) Resolve(
	ctx context.Context,
	files []server.FileReference,
) (server.ResolvedFiles, error) {
	refused := func(err error) (server.ResolvedFiles, error) {
		code := string(webapi.ErrorCodeFrameDeliveryFailed)
		return server.ResolvedFiles{}, &server.ContentError{Message: err.Error(), Code: &code, Cause: err}
	}
	resolved := make([][]core.UserContentBlock, len(files))
	var media store.HeldMedia
	var remote []hostaccess.RemoteFile
	var positions []int
	for i, file := range files {
		switch file := file.(type) {
		case *server.Upload:
			if f.host == nil {
				//nolint:staticcheck // Product text is copied verbatim from Rust.
				return refused(errors.New("The frame's Host was not admitted"))
			}
			blocks, held, err := hostaccess.ResolveUpload(ctx, f.shard, f.conversation, f.host, file.Ref, file.FileName)
			if err != nil {
				return refused(err)
			}
			resolved[i] = blocks
			media.Absorb(held)
		case *server.RemoteFile:
			remote = append(remote, hostaccess.RemoteFile{Device: file.DeviceID, Path: file.Path})
			positions = append(positions, i)
		}
	}
	if len(remote) > 0 {
		references, err := hostaccess.ReferenceRemoteFiles(ctx, f.shard, f.conversation, remote)
		if err != nil {
			return refused(err)
		}
		for i, reference := range references {
			resolved[positions[i]] = []core.UserContentBlock{reference}
		}
	}
	return server.ResolvedFiles{Blocks: resolved, Media: media}, nil
}

func addedBlocks(patches []framewire.TranscriptPatch) []core.Block {
	var blocks []core.Block
	for _, patch := range patches {
		switch patch := patch.(type) {
		case *framewire.AddPatch:
			blocks = append(blocks, patch.Value)
		case *framewire.ReplaceBlockPatch:
			blocks = append(blocks, patch.Value)
		case *framewire.ReplacePatch:
			blocks = append(blocks, patch.Value...)
		case *framewire.AppendTextPatch:
		}
	}
	return blocks
}

func (s *Shard) present(ctx context.Context, frame framewire.ServerFrame) (framewire.ServerFrame, error) {
	var blocks []core.Block
	var destination **framewire.Failures
	switch original := frame.(type) {
	case *framewire.TranscriptResetFrame:
		presented := *original
		frame = &presented
		blocks = presented.Blocks
		destination = &presented.Failures
	case *framewire.TranscriptPatchFrame:
		presented := *original
		frame = &presented
		blocks = addedBlocks(presented.Patches)
		destination = &presented.Failures
	case *framewire.SubagentTranscriptResetFrame:
		presented := *original
		frame = &presented
		blocks = presented.Blocks
		destination = &presented.Failures
	case *framewire.SubagentTranscriptPatchFrame:
		presented := *original
		frame = &presented
		blocks = addedBlocks(presented.Patches)
		destination = &presented.Failures
	case *framewire.AbortResultFrame,
		*framewire.ClosedFrame,
		*framewire.EditResultFrame,
		*framewire.ErrorFrame,
		*framewire.HeartbeatFrame,
		*framewire.OpenedFrame,
		*framewire.PendingSteersFrame,
		*framewire.PhaseFrame,
		*framewire.QueueFrame,
		*framewire.RejectedFrame,
		*framewire.RetryScheduledFrame,
		*framewire.ShellOutputFrame,
		*framewire.ShellWriteResultFrame,
		*framewire.SteerResultFrame,
		*framewire.SubagentFrame:
		return frame, nil
	}
	facts, err := FailureFacts(ctx, s.services.Assembly, blocks)
	if facts != nil {
		*destination = &facts
	} else {
		*destination = nil
	}
	return frame, err
}

// prepareOpen checks that a visible model is selected before marking the conversation live.
func (s *Shard) prepareOpen(ctx context.Context, record database.ConversationRecord) error {
	if record.Model == nil {
		return &frameRefusalError{
			webapi.ErrorCodeModelNotSelected,
			"Choose a model for the conversation before opening it",
		}
	}
	id, err := webapi.ParseProviderID(record.Model.ProviderID)
	visible := false
	if err == nil {
		entry, err := s.services.Vault.Visible(ctx, s.user, id)
		if err != nil {
			return err
		}
		visible = entry != nil
	}
	if !visible {
		return &frameRefusalError{webapi.ErrorCodeProviderNotFound, "No such provider"}
	}
	now := s.Clock().Now()
	return s.Control().MarkLive(ctx, record.ID, now)
}

// prepareSend records a user message and starts its first-message title request.
func (s *Shard) prepareSend(
	ctx context.Context,
	record database.ConversationRecord,
	frame *framewire.SendFrame,
) error {
	id := record.ID
	seen, err := s.Control().CountUserMessage(ctx, record.ID)
	if err != nil {
		return err
	}
	var text string
	for _, part := range frame.Content {
		if part, ok := part.(*framewire.TextContent); ok {
			text = part.Text
			break
		}
	}
	title := server.TitleFromMessage(text)
	if title != "" {
		titled, err := s.Control().TitleFromFirstMessage(ctx, id, title)
		if err != nil {
			return err
		}
		if titled && record.Model != nil {
			s.startTitle(id, *record.Model, titleRequest{messages: []string{text}, from: title, seen: seen})
		}
	}
	if err := s.Control().TouchConversation(ctx, id); err != nil {
		return err
	}
	s.Mark(pagesync.Part{Kind: pagesync.Conversation, ConversationID: id})
	return nil
}

// uploadFailureCode preserves host access refusal codes for upload frames.
func uploadFailureCode(err error) webapi.ErrorCode {
	code := webapi.ErrorCodeFrameDeliveryFailed
	var access *hostaccess.Error
	if errors.As(err, &access) {
		code, _ = access.Code()
	}
	return code
}
