package edge

import (
	"context"
	"database/sql"
	"fmt"
	"net/http"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/backend/usershard"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

func (e *Edge) owned(r *http.Request) (*database.ConversationRecord, error) {
	missing := apiFailure(404, "conversation_not_found", "No such conversation")
	id, err := webapi.ParseConversationID(r.PathValue("id"))
	if err != nil {
		return nil, missing
	}
	record, err := e.state.Services.Control.Conversation(r.Context(), id)
	if err != nil {
		return nil, err
	}
	if record == nil || record.Owner != caller(r).ID {
		return nil, missing
	}
	return record, nil
}

func (e *Edge) conversations(w http.ResponseWriter, r *http.Request) error {
	query, err := decodeQuery(r, webapi.DecodeConversationsQuery, "archived")
	if err != nil {
		return apiFailure(400, "invalid_query", err.Error())
	}
	shard, err := e.state.Shards.Of(r.Context(), caller(r).ID)
	if err != nil {
		return err
	}
	result, err := shard.ConversationSummaries(r.Context(), bool(query.Archived))
	if err != nil {
		return err
	}
	writeJSON(w, 200, webapi.Conversations{Conversations: result})
	return nil
}

func (e *Edge) createConversation(w http.ResponseWriter, r *http.Request) error {
	request, err := decodeBody(r, webapi.DecodeCreateConversation)
	if err != nil {
		return err
	}
	result, err := e.state.Services.Control.CreateConversation(r.Context(), caller(r).ID, request.ID)
	if err != nil {
		return err
	}
	var record database.ConversationRecord
	status := 200
	switch result := result.(type) {
	case *database.ConversationCreated:
		record = result.Record
		status = 201
		e.state.Services.Sync.Mark(caller(r).ID, pagesync.Part{Kind: pagesync.Conversation, ConversationID: record.ID})
		e.state.Services.Sync.Mark(caller(r).ID, pagesync.Part{Kind: pagesync.ConversationOrder})
	case *database.ConversationExisting:
		record = result.Record
	case *database.ConversationUnavailable:
		return apiFailure(409, "id_unavailable", "Conversation id is unavailable")
	}
	shard, err := e.state.Shards.Of(r.Context(), caller(r).ID)
	if err != nil {
		return err
	}
	summary, err := shard.ConversationSummary(r.Context(), record)
	if err != nil {
		return err
	}
	writeJSON(w, status, webapi.ConversationAnswer{Conversation: summary})
	return nil
}

func (e *Edge) patchConversation(w http.ResponseWriter, r *http.Request) error {
	request, err := decodeBody(r, webapi.DecodeConversationPatch)
	if err != nil {
		return err
	}
	id, err := webapi.ParseConversationID(r.PathValue("id"))
	if err != nil {
		return apiFailure(404, "conversation_not_found", "No such conversation")
	}
	shard, err := e.state.Shards.Of(r.Context(), caller(r).ID)
	if err != nil {
		return err
	}
	update, err := shard.ApplyPatch(r.Context(), id, request)
	if err != nil {
		return err
	}
	if update == nil {
		return apiFailure(404, "conversation_not_found", "No such conversation")
	}
	status := 200
	for _, result := range update.Results {
		switch result := result.(type) {
		case *webapi.FieldResultApplied:
		case *webapi.FieldResultFailed:
			if len(update.Results) == 1 {
				return apiFailure(int(result.HTTPStatus), result.Code, result.Message)
			}
			status = 207
		}
	}
	writeJSON(w, status, update)
	return nil
}

func (e *Edge) title(w http.ResponseWriter, r *http.Request) error {
	id, err := webapi.ParseConversationID(r.PathValue("id"))
	if err != nil {
		return apiFailure(404, "conversation_not_found", "No such conversation")
	}
	shard, err := e.state.Shards.Of(r.Context(), caller(r).ID)
	if err != nil {
		return err
	}
	if err := shard.AskTitle(r.Context(), id); err != nil {
		return err
	}
	w.WriteHeader(202)
	return nil
}

func (e *Edge) fork(w http.ResponseWriter, r *http.Request) error {
	request, err := decodeBody(r, webapi.DecodeForkRequest)
	if err != nil {
		return err
	}
	id, err := webapi.ParseConversationID(r.PathValue("id"))
	if err != nil {
		return apiFailure(404, "conversation_not_found", "No such conversation")
	}
	shard, err := e.state.Shards.Of(r.Context(), caller(r).ID)
	if err != nil {
		return err
	}
	forked, err := shard.Fork(r.Context(), id, request.ID, request.BlockID)
	if err != nil {
		return err
	}
	summary, err := shard.ConversationSummary(r.Context(), forked.Record)
	if err != nil {
		return err
	}
	status := 200
	if forked.Created {
		status = 201
	}
	writeJSON(w, status, webapi.ForkAnswer{Conversation: summary})
	return nil
}

func (e *Edge) batch(w http.ResponseWriter, r *http.Request) error {
	request, err := decodeBody(r, webapi.DecodeConversationBatch)
	if err != nil {
		return err
	}
	shard, err := e.state.Shards.Of(r.Context(), caller(r).ID)
	if err != nil {
		return err
	}
	results := make([]webapi.BatchResult, 0, len(request.Items))
	for _, item := range request.Items {
		update, err := shard.ApplyPatch(r.Context(), item.ID, item.Patch)
		if err != nil {
			return err
		}
		if update == nil {
			results = append(
				results,
				&webapi.BatchResultRefused{
					ID:      item.ID,
					Code:    "conversation_not_found",
					Message: "No such conversation",
				},
			)
		} else {
			results = append(
				results,
				&webapi.BatchResultUpdated{ID: item.ID, Conversation: update.Conversation, Results: update.Results},
			)
		}
	}
	writeJSON(w, 207, webapi.BatchAnswer{Results: results})
	return nil
}

func (e *Edge) transcript(w http.ResponseWriter, r *http.Request) error {
	record, err := e.owned(r)
	if err != nil {
		return err
	}
	history := database.History{Blocks: []core.Block{}}
	_, err = e.state.Services.Conversations.Read(r.Context(), record.ID, func(ctx context.Context, tx *sql.Tx) error {
		var err error
		history, err = database.ReadHistory(ctx, tx)
		return err
	})
	if err != nil {
		return err
	}
	failures, err := usershard.FailureFacts(r.Context(), e.state.Services.Assembly, history.Blocks)
	if err != nil {
		return err
	}
	result := webapi.Transcript{
		Blocks:    history.Blocks,
		Failures:  &failures,
		Subagents: make([]webapi.SubagentHistory, 0, len(history.Subagents)),
	}
	for _, node := range history.Subagents {
		job := node.Record.Job()
		if job == nil {
			return fmt.Errorf("subagent %s has no parent", node.Record.ID)
		}
		failures, err := usershard.FailureFacts(r.Context(), e.state.Services.Assembly, node.Blocks)
		if err != nil {
			return err
		}
		result.Subagents = append(
			result.Subagents,
			webapi.SubagentHistory{Subagent: *job, Blocks: node.Blocks, Failures: &failures},
		)
	}
	if len(*result.Failures) == 0 {
		result.Failures = nil
	}
	for i := range result.Subagents {
		if len(*result.Subagents[i].Failures) == 0 {
			result.Subagents[i].Failures = nil
		}
	}
	writeJSON(w, 200, result)
	return nil
}

func (e *Edge) readConversation(w http.ResponseWriter, r *http.Request) error {
	request, err := decodeBody(r, webapi.DecodeReadRequest)
	if err != nil {
		return err
	}
	record, err := e.owned(r)
	if err != nil {
		return err
	}
	facts := database.EmptySummary()
	_, err = e.state.Services.Conversations.Read(r.Context(), record.ID, func(ctx context.Context, tx *sql.Tx) error {
		var err error
		facts, err = database.Summary(ctx, tx)
		return err
	})
	if err != nil {
		return err
	}
	if request.Revision > facts.Revision {
		return apiFailure(409, "invalid_revision", "Cannot read beyond current output")
	}
	if err := e.state.Services.Control.MarkConversationRead(r.Context(), record.ID, request.Revision); err != nil {
		return err
	}
	e.state.Services.Sync.Mark(caller(r).ID, pagesync.Part{Kind: pagesync.Conversation, ConversationID: record.ID})
	w.WriteHeader(204)
	return nil
}
