package database

import (
	"context"
	"database/sql"
	"encoding/json"
	"fmt"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/core"
)

// Each version holds the node's complete command-storage map.
// +demi:root
type commandValues map[store.CommandStorageKey]json.RawMessage

// writeCommandState preserves immutable command versions and replaces their boundaries.
func writeCommandState(ctx context.Context, tx *sql.Tx, node core.NodeID, state store.CommandStateSnapshot) error {
	revisions := make([]uint64, 0, len(state.Versions))
	for _, version := range state.Versions {
		if err := writeCommandVersion(ctx, tx, node, version); err != nil {
			return err
		}
		revisions = append(revisions, version.Revision)
	}
	if err := execSQL(ctx, tx, "DELETE FROM session_boundaries WHERE node_id=?", node); err != nil {
		return err
	}
	keep, err := encoded(revisions)
	if err != nil {
		return err
	}
	if err := execSQL(
		ctx,
		tx,
		"DELETE FROM command_snapshots WHERE node_id=? AND revision NOT IN (SELECT value FROM json_each(?))",
		node,
		keep,
	); err != nil {
		return err
	}
	for _, boundary := range state.Boundaries {
		if err := execSQL(
			ctx,
			tx,
			"INSERT INTO session_boundaries (node_id,block_id,edge,command_revision) VALUES (?,?,?,?)",
			node,
			boundary.BlockID,
			boundary.Edge,
			boundary.CommandRevision,
		); err != nil {
			return err
		}
	}
	return nil
}

func readCommandState(
	ctx context.Context,
	tx *sql.Tx,
	node core.NodeID,
	revision uint64,
) (store.CommandStateSnapshot, error) {
	versions, err := queryRecords(
		ctx,
		tx,
		"command_snapshots",
		"SELECT revision,entries FROM command_snapshots WHERE node_id=? ORDER BY revision",
		func(r *storedRow) store.CommandVersion {
			return store.CommandVersion{
				Revision: r.count("revision"),
				Values:   storedJSON(r, "entries", decodeCommandValues),
			}
		},
		node,
	)
	if err != nil {
		return store.CommandStateSnapshot{}, err
	}
	boundaries, err := queryRecords(
		ctx,
		tx,
		"session_boundaries",
		"SELECT block_id,edge,command_revision FROM session_boundaries WHERE node_id=? ORDER BY block_id,edge",
		func(r *storedRow) store.SessionBoundary {
			edge := store.BoundaryEdge(r.text("edge"))
			r.bad("edge", edge.Validate())
			return store.SessionBoundary{
				BlockID:         checked(r, "block_id", core.ParseBlockID),
				Edge:            edge,
				CommandRevision: r.count("command_revision"),
			}
		},
		node,
	)
	return store.CommandStateSnapshot{Revision: revision, Versions: versions, Boundaries: boundaries}, err
}

func writeCommandVersion(ctx context.Context, tx *sql.Tx, node core.NodeID, version store.CommandVersion) error {
	before, found, err := queryRecord(
		ctx,
		tx,
		"command_snapshots",
		"SELECT entries FROM command_snapshots WHERE node_id=? AND revision=?",
		func(r *storedRow) commandValues { return storedJSON(r, "entries", decodeCommandValues) },
		node,
		version.Revision,
	)
	if err != nil {
		return err
	}
	if !found {
		document, err := encoded(commandValues(version.Values))
		if err != nil {
			return err
		}
		if err := execSQL(
			ctx,
			tx,
			"INSERT INTO command_snapshots (node_id,revision,entries) VALUES (?,?,?)",
			node,
			version.Revision,
			document,
		); err != nil {
			return err
		}
		return nil
	}
	same, err := equalCommandValues(before, version.Values)
	if err != nil {
		return err
	}
	if !same {
		return fmt.Errorf("command-state version %d is immutable", version.Revision)
	}
	return nil
}
