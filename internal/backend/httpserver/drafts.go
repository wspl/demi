package httpserver

import (
	"fmt"
	"net/http"
	"strings"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/conversationproto"
	"github.com/wspl/demi/internal/webapiproto"
)

func (e *Server) draft(w http.ResponseWriter, r *http.Request) error {
	record, err := e.owned(r)
	if err != nil {
		return err
	}
	if r.Method == "GET" || r.Method == "HEAD" {
		draft, err := e.state.Services.Control.Draft(r.Context(), record.ID)
		if err != nil {
			return err
		}
		writeJSON(w, 200, webapiproto.DraftAnswer{Draft: draft})
		return nil
	}
	request, err := decodeBody(r, webapiproto.DecodeDraftSave)
	if err != nil {
		return err
	}
	count := strings.Count(request.Text, string(webapiproto.AttachmentMark))
	if count != len(request.Files) {
		return apiFailure(
			400,
			"invalid_body",
			fmt.Sprintf("text: holds %d attachment marks for %d files", count, len(request.Files)),
		)
	}
	files := make([]database.StagedFile, 0, len(request.Files))
	for index, file := range request.Files {
		switch file := file.(type) {
		case *conversationproto.UploadContent:
			id, err := webapiproto.ParseAttachmentID(file.Ref)
			if err != nil {
				return apiFailure(400, "invalid_body", fmt.Sprintf("files[%d].ref: %v", index, err))
			}
			files = append(files, &database.StagedUpload{ID: id, FileName: file.FileName})
		case *conversationproto.RemoteFileContent:
			files = append(files, &database.StagedRemote{DeviceID: file.DeviceID, Path: file.Path})
		case *conversationproto.TextContent,
			*conversationproto.ReferenceContent,
			*conversationproto.MediaContent,
			*conversationproto.AttachmentContent:
			return apiFailure(
				400,
				"invalid_body",
				fmt.Sprintf("files[%d]: a draft's file is an upload or a remote file", index),
			)
		}
	}
	saved, err := e.state.Services.Control.SaveDraft(
		r.Context(),
		record.ID,
		caller(r).ID,
		request.Base,
		request.Text,
		files,
	)
	if err != nil {
		return err
	}
	e.state.Services.Sync.Mark(caller(r).ID, pagesync.Part{Kind: pagesync.Conversation, ConversationID: record.ID})
	writeJSON(w, 200, webapiproto.DraftAnswer{Draft: saved})
	return nil
}

func (e *Server) replacedDraft(w http.ResponseWriter, r *http.Request) error {
	record, err := e.owned(r)
	if err != nil {
		return err
	}
	request, err := decodeBody(r, webapiproto.DecodeReplacedDraftAction)
	if err != nil {
		return err
	}
	saved, err := e.state.Services.Control.ChangeReplacedDraft(r.Context(), record.ID, request.Action, request.Revision)
	if err != nil {
		return err
	}
	e.state.Services.Sync.Mark(caller(r).ID, pagesync.Part{Kind: pagesync.Conversation, ConversationID: record.ID})
	writeJSON(w, 200, webapiproto.DraftAnswer{Draft: saved})
	return nil
}

func (e *Server) reorder(w http.ResponseWriter, r *http.Request) error {
	request, err := decodeBody(r, webapiproto.DecodeSidebarReorder)
	if err != nil {
		return err
	}
	var moved bool
	var kind pagesync.Kind
	switch request := request.(type) {
	case *webapiproto.SidebarReorderConversation:
		moved, err = e.state.Services.Control.ReorderConversations(
			r.Context(),
			caller(r).ID,
			request.ID,
			request.BeforeID,
		)
		kind = pagesync.ConversationOrder
	case *webapiproto.SidebarReorderWorkspace:
		moved, err = e.state.Services.Control.ReorderWorkspaces(r.Context(), caller(r).ID, request.ID, request.BeforeID)
		kind = pagesync.Workspaces
	}
	if err != nil {
		return err
	}
	if !moved {
		return apiFailure(409, "invalid_order", "Rows must belong to the same project and pin partition")
	}
	e.state.Services.Sync.Mark(caller(r).ID, pagesync.Part{Kind: kind})
	w.WriteHeader(204)
	return nil
}
