package edge

import (
	"fmt"
	"net/http"
	"strings"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/webapi"
)

func (e *Edge) draft(w http.ResponseWriter, r *http.Request) error {
	record, err := e.owned(r)
	if err != nil {
		return err
	}
	if r.Method == "GET" || r.Method == "HEAD" {
		draft, err := e.state.Services.Control.Draft(r.Context(), record.ID)
		if err != nil {
			return err
		}
		writeJSON(w, 200, webapi.DraftAnswer{Draft: draft})
		return nil
	}
	request, err := decodeBody(r, webapi.DecodeDraftSave)
	if err != nil {
		return err
	}
	count := strings.Count(request.Text, string(webapi.AttachmentMark))
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
		case *framewire.UploadContent:
			id, err := webapi.ParseAttachmentID(file.Ref)
			if err != nil {
				return apiFailure(400, "invalid_body", fmt.Sprintf("files[%d].ref: %v", index, err))
			}
			files = append(files, &database.StagedUpload{ID: id, FileName: file.FileName})
		case *framewire.RemoteFileContent:
			files = append(files, &database.StagedRemote{DeviceID: file.DeviceID, Path: file.Path})
		case *framewire.TextContent, *framewire.ReferenceContent, *framewire.MediaContent, *framewire.AttachmentContent:
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
	writeJSON(w, 200, webapi.DraftAnswer{Draft: saved})
	return nil
}

func (e *Edge) replacedDraft(w http.ResponseWriter, r *http.Request) error {
	record, err := e.owned(r)
	if err != nil {
		return err
	}
	request, err := decodeBody(r, webapi.DecodeReplacedDraftAction)
	if err != nil {
		return err
	}
	saved, err := e.state.Services.Control.ChangeReplacedDraft(r.Context(), record.ID, request.Action, request.Revision)
	if err != nil {
		return err
	}
	e.state.Services.Sync.Mark(caller(r).ID, pagesync.Part{Kind: pagesync.Conversation, ConversationID: record.ID})
	writeJSON(w, 200, webapi.DraftAnswer{Draft: saved})
	return nil
}

func (e *Edge) reorder(w http.ResponseWriter, r *http.Request) error {
	request, err := decodeBody(r, webapi.DecodeSidebarReorder)
	if err != nil {
		return err
	}
	var moved bool
	var kind pagesync.Kind
	switch request := request.(type) {
	case *webapi.SidebarReorderConversation:
		moved, err = e.state.Services.Control.ReorderConversations(
			r.Context(),
			caller(r).ID,
			request.ID,
			request.BeforeID,
		)
		kind = pagesync.ConversationOrder
	case *webapi.SidebarReorderWorkspace:
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
