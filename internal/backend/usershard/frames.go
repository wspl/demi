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
	"github.com/wspl/demi/internal/conversationproto"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

type frameRefusalError struct {
	code    webapiproto.ErrorCode
	message string
}

// Error returns the frame refusal text shown to the client.
func (r *frameRefusalError) Error() string {
	return r.message
}

func (s *Shard) prepareFrame(
	ctx context.Context,
	id webapiproto.ConversationID,
	frame conversationproto.ClientFrame,
) error {
	record, found, err := s.Control().Conversation(ctx, id)
	if err != nil {
		return err
	}
	if !found || record.Owner != s.user {
		return &frameRefusalError{
			webapiproto.ErrorCodeConversationNotFound,
			"No such conversation",
		}
	}
	if record.Archived && frame.Kind() != conversationproto.ClientFrameKindClose {
		return &frameRefusalError{
			webapiproto.ErrorCodeConversationArchived,
			"Restore the conversation before writing to it",
		}
	}
	switch frame := frame.(type) {
	case *conversationproto.OpenFrame:
		return s.prepareOpen(ctx, record)
	case *conversationproto.SendFrame:
		return s.prepareSend(ctx, record, frame)
	case *conversationproto.SteerFrame, *conversationproto.EditAndSendFrame:
		if _, err := s.Control().CountUserMessage(ctx, id); err != nil {
			return err
		}
		s.Mark(pagesync.Part{Kind: pagesync.Conversation, ConversationID: id})
	case *conversationproto.AbortFrame,
		*conversationproto.AbortSubagentFrame,
		*conversationproto.AbortSubagentsFrame,
		*conversationproto.CancelPendingSteerFrame,
		*conversationproto.ClearMessageQueueFrame,
		*conversationproto.CloseFrame,
		*conversationproto.CompactFrame,
		*conversationproto.DequeueMessageFrame,
		*conversationproto.ResumeFrame,
		*conversationproto.RetryFrame,
		*conversationproto.SendQueuedMessageFrame,
		*conversationproto.ShellAbortFrame,
		*conversationproto.ShellWriteFrame,
		*conversationproto.SteerQueuedMessageFrame,
		*conversationproto.SyncTranscriptFrame:
	}
	return nil
}

func (s *Shard) restoreTree(ctx context.Context, id webapiproto.ConversationID) error {
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
	if err := s.prepareFrame(ctx, id, &conversationproto.OpenFrame{}); err != nil {
		return err
	}
	return s.agent.Restore(ctx, hostaccess.RootOf(id), database.ExecutionPath(target))
}

func refusedFrame(
	frame conversationproto.ClientFrame,
	code webapiproto.ErrorCode,
	message string,
) conversationproto.ServerFrame {
	if edit, ok := frame.(*conversationproto.EditAndSendFrame); ok {
		return &conversationproto.EditResultFrame{
			OperationID: edit.Request.OperationID,
			Outcome:     &conversationproto.RejectedEdit{Reason: message},
		}
	}
	name := string(code)
	return &conversationproto.ErrorFrame{Message: message, Code: &name}
}

func carriesUploads(frame conversationproto.ClientFrame) bool {
	var content []conversationproto.ClientContent
	switch frame := frame.(type) {
	case *conversationproto.SendFrame:
		content = frame.Content
	case *conversationproto.SteerFrame:
		content = frame.Content
	case *conversationproto.EditAndSendFrame:
		content = frame.Request.Content
	case *conversationproto.AbortFrame,
		*conversationproto.AbortSubagentFrame,
		*conversationproto.AbortSubagentsFrame,
		*conversationproto.CancelPendingSteerFrame,
		*conversationproto.ClearMessageQueueFrame,
		*conversationproto.CloseFrame,
		*conversationproto.CompactFrame,
		*conversationproto.DequeueMessageFrame,
		*conversationproto.OpenFrame,
		*conversationproto.ResumeFrame,
		*conversationproto.RetryFrame,
		*conversationproto.SendQueuedMessageFrame,
		*conversationproto.ShellAbortFrame,
		*conversationproto.ShellWriteFrame,
		*conversationproto.SteerQueuedMessageFrame,
		*conversationproto.SyncTranscriptFrame:
	}
	for _, part := range content {
		if _, ok := part.(*conversationproto.UploadContent); ok {
			return true
		}
	}
	return false
}

func (s *Shard) handleMessage(
	ctx context.Context,
	id webapiproto.ConversationID,
	connection *server.Connection[*remotehost.Host],
	files *conversationFiles,
	text []byte,
) (conversationproto.ServerFrame, error) {
	frame, err := conversationproto.DecodeClientFrame(text)
	if errors.Is(err, conversationproto.ErrNotJSON) {
		return nil, err
	}
	if err != nil {
		return refusedFrame(nil, webapiproto.ErrorCodeInvalidFrame, fmt.Sprintf("Invalid client frame: %v", err)), nil
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
	if frame.Kind() == conversationproto.ClientFrameKindOpen {
		settings, err := s.conversations.Slot(id).Settings().Acquire(ctx)
		if err != nil {
			return nil, err
		}
		defer settings.Release()
	}
	if err := s.prepareFrame(ctx, id, frame); err != nil {
		code := webapiproto.ErrorCodeFrameDeliveryFailed
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
	conversation webapiproto.ConversationID
	host         *hostaccess.ConversationHost
}

// Resolve admits uploaded and remote files as conversation content.
func (f *conversationFiles) Resolve(
	ctx context.Context,
	files []server.FileReference,
) (server.ResolvedFiles, error) {
	refused := func(err error) (server.ResolvedFiles, error) {
		code := string(webapiproto.ErrorCodeFrameDeliveryFailed)
		return server.ResolvedFiles{}, &server.ContentError{Message: err.Error(), Code: &code, Cause: err}
	}
	resolved := make([][]types.UserContentBlock, len(files))
	var media store.HeldMedia
	var remote []hostaccess.RemoteFile
	var positions []int
	for i, file := range files {
		switch file := file.(type) {
		case *server.Upload:
			if f.host == nil {
				//nolint:staticcheck // Product text, shown to the user as it is.
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
			resolved[positions[i]] = []types.UserContentBlock{reference}
		}
	}
	return server.ResolvedFiles{Blocks: resolved, Media: media}, nil
}

func addedBlocks(patches []conversationproto.TranscriptPatch) []types.Block {
	var blocks []types.Block
	for _, patch := range patches {
		switch patch := patch.(type) {
		case *conversationproto.AddPatch:
			blocks = append(blocks, patch.Value)
		case *conversationproto.ReplaceBlockPatch:
			blocks = append(blocks, patch.Value)
		case *conversationproto.ReplacePatch:
			blocks = append(blocks, patch.Value...)
		case *conversationproto.AppendTextPatch:
		}
	}
	return blocks
}

func (s *Shard) present(
	ctx context.Context,
	frame conversationproto.ServerFrame,
) (conversationproto.ServerFrame, error) {
	var blocks []types.Block
	var destination **conversationproto.Failures
	switch original := frame.(type) {
	case *conversationproto.TranscriptResetFrame:
		presented := *original
		frame = &presented
		blocks = presented.Blocks
		destination = &presented.Failures
	case *conversationproto.TranscriptPatchFrame:
		presented := *original
		frame = &presented
		blocks = addedBlocks(presented.Patches)
		destination = &presented.Failures
	case *conversationproto.SubagentTranscriptResetFrame:
		presented := *original
		frame = &presented
		blocks = presented.Blocks
		destination = &presented.Failures
	case *conversationproto.SubagentTranscriptPatchFrame:
		presented := *original
		frame = &presented
		blocks = addedBlocks(presented.Patches)
		destination = &presented.Failures
	case *conversationproto.AbortResultFrame,
		*conversationproto.ClosedFrame,
		*conversationproto.EditResultFrame,
		*conversationproto.ErrorFrame,
		*conversationproto.HeartbeatFrame,
		*conversationproto.OpenedFrame,
		*conversationproto.PendingSteersFrame,
		*conversationproto.PhaseFrame,
		*conversationproto.QueueFrame,
		*conversationproto.RejectedFrame,
		*conversationproto.RetryScheduledFrame,
		*conversationproto.ShellOutputFrame,
		*conversationproto.ShellWriteResultFrame,
		*conversationproto.SteerResultFrame,
		*conversationproto.SubagentFrame:
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
			webapiproto.ErrorCodeModelNotSelected,
			"Choose a model for the conversation before opening it",
		}
	}
	id, err := webapiproto.ParseProviderID(record.Model.ProviderID)
	visible := false
	if err == nil {
		entry, err := s.services.Vault.Visible(ctx, s.user, id)
		if err != nil {
			return err
		}
		visible = entry != nil
	}
	if !visible {
		return &frameRefusalError{webapiproto.ErrorCodeProviderNotFound, "No such provider"}
	}
	now := s.Clock().Now()
	return s.Control().MarkLive(ctx, record.ID, now)
}

// prepareSend records a user message and starts its first-message title request.
func (s *Shard) prepareSend(
	ctx context.Context,
	record database.ConversationRecord,
	frame *conversationproto.SendFrame,
) error {
	id := record.ID
	seen, err := s.Control().CountUserMessage(ctx, record.ID)
	if err != nil {
		return err
	}
	var text string
	for _, part := range frame.Content {
		if part, ok := part.(*conversationproto.TextContent); ok {
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
func uploadFailureCode(err error) webapiproto.ErrorCode {
	code := webapiproto.ErrorCodeFrameDeliveryFailed
	var access *hostaccess.Error
	if errors.As(err, &access) {
		code, _ = access.Code()
	}
	return code
}
