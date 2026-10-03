package database

import (
	"context"
	"database/sql"
	"encoding/json"
	"reflect"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/webapi"
)

// UserPlugins returns each plugin `user` turned on or off, by id; one the user never
// switched is on.
func (c *ControlService) UserPlugins(ctx context.Context, user webapi.UserID) (map[string]bool, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) (map[string]bool, error) {
		choices := make(map[string]bool)
		_, err := queryRecords(
			ctx,
			tx,
			"user_plugins",
			"SELECT plugin,enabled FROM user_plugins WHERE user_id = ?",
			func(r *storedRow) struct{} {
				choices[r.text("plugin")] = r.boolean("enabled")
				return struct{}{}
			},
			user,
		)
		return choices, err
	})
}

// SetUserPlugin records that `user` has `plugin` on or off.
func (c *ControlService) SetUserPlugin(ctx context.Context, user webapi.UserID, plugin string, enabled bool) error {
	return controlDo(ctx, c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) error {
		return execSQL(
			ctx,
			tx,
			`INSERT INTO user_plugins (user_id,plugin,enabled)
VALUES (?,?,?)
ON CONFLICT (user_id,plugin) DO UPDATE
SET enabled=excluded.enabled`,
			user,
			plugin,
			enabled,
		)
	})
}

// PluginValue returns `plugin`'s value `key` for `user`.
func (c *ControlService) PluginValue(
	ctx context.Context,
	user webapi.UserID,
	plugin string,
	key string,
) (PluginValue, bool, error) {
	var found bool
	record, err := controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) (PluginValue, error) {
		r, ok, err := queryRecord(
			ctx,
			tx,
			"plugin_values",
			"SELECT document,revision FROM plugin_values WHERE user_id = ? AND plugin = ? AND key = ?",
			pluginValueRow,
			user,
			plugin,
			key,
		)
		found = ok
		return r, err
	})
	return record, found && err == nil, err
}

// PluginValues returns every value of `plugin`'s for `user`, by key.
func (c *ControlService) PluginValues(
	ctx context.Context,
	user webapi.UserID,
	plugin string,
) (map[string]PluginValue, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) (map[string]PluginValue, error) {
		values := make(map[string]PluginValue)
		_, err := queryRecords(
			ctx,
			tx,
			"plugin_values",
			"SELECT key,document,revision FROM plugin_values WHERE user_id = ? AND plugin = ?",
			func(r *storedRow) struct{} {
				values[r.text("key")] = pluginValueRow(r)
				return struct{}{}
			},
			user,
			plugin,
		)
		return values, err
	})
}

// WritePluginValue writes `plugin`'s value `key` for `user` if it is still at
// `revision`, none for a value that does not exist yet, in one
// transaction, so two writes never build on the same revision. The
// value names `blobs`; the blobs it named before and names now are
// used before it commits (`storage.md` § Collecting blobs).
func (c *ControlService) WritePluginValue(ctx context.Context, write ValueWrite, uses OwnerBlobs) (uint64, error) {
	var refused error
	revision, err := controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) (uint64, error) {
		var revision uint64
		var err error
		revision, refused, err = writePluginValueTx(ctx, tx, write, uses)
		return revision, err
	})
	if err != nil {
		return 0, err
	}
	if refused != nil {
		return 0, refused
	}
	return revision, nil
}

// writePluginValueTx performs one plugin value write inside tx. refused
// is why nothing was written (the plugin sees its text unprefixed);
// err fails the transaction.
func writePluginValueTx(
	ctx context.Context,
	tx *sql.Tx,
	write ValueWrite,
	uses OwnerBlobs,
) (revision uint64, refused, err error) {
	text, err := encoded(write.Document)
	if err != nil {
		return 0, nil, err
	}
	named, err := encoded(blobNames(write.Blobs))
	if err != nil {
		return 0, nil, err
	}
	before, found, err := queryRecord(
		ctx,
		tx,
		"plugin_values",
		"SELECT revision,blobs FROM plugin_values WHERE user_id = ? AND plugin = ? AND key = ?",
		valueBlobRow,
		write.User,
		write.Plugin,
		write.Key,
	)
	if err != nil {
		return 0, nil, err
	}
	var stored *uint64
	if found {
		stored = &before.revision
	}
	if !reflect.DeepEqual(stored, write.Revision) {
		return 0, nil, ErrRevisionConflict
	}
	touched := append([]core.BlobRef{}, write.Blobs...)
	if found {
		touched = append(touched, before.blobs...)
	}
	if err := uses.CommitUses(ctx, touched); err != nil {
		return 0, err, nil
	}
	revision = uint64(1)
	if write.Revision != nil {
		revision = *write.Revision + 1
	}
	err = execSQL(
		ctx,
		tx,
		`INSERT INTO plugin_values (user_id,plugin,key,document,revision,blobs)
VALUES (?,?,?,?,?,?)
ON CONFLICT (user_id,plugin,key) DO UPDATE
SET document=excluded.document,revision=excluded.revision,blobs=excluded.blobs`,
		write.User,
		write.Plugin,
		write.Key,
		text,
		integer(revision),
		named,
	)
	return revision, nil, err
}

// RemovePluginValue removes `plugin`'s value `key` for `user` if it is still at
// `revision`, in one transaction; the blobs it named are used before
// it commits.
func (c *ControlService) RemovePluginValue(
	ctx context.Context,
	user webapi.UserID,
	plugin string,
	key string,
	revision uint64,
	uses OwnerBlobs,
) error {
	var refused error
	err := controlDo(ctx, c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) error {
		before, found, err := queryRecord(
			ctx,
			tx,
			"plugin_values",
			"SELECT revision,blobs FROM plugin_values WHERE user_id = ? AND plugin = ? AND key = ?",
			valueBlobRow,
			user,
			plugin,
			key,
		)
		if err != nil {
			return err
		}
		if !found || before.revision != revision {
			return ErrRevisionConflict
		}
		if err := uses.CommitUses(ctx, before.blobs); err != nil {
			refused = err
			return nil
		}
		return execSQL(
			ctx,
			tx,
			"DELETE FROM plugin_values WHERE user_id = ? AND plugin = ? AND key = ?",
			user,
			plugin,
			key,
		)
	})
	if err != nil {
		return err
	}
	return refused
}

// PluginDirectories returns every Host directory of each of `user`'s plugins, by plugin id, each
// set in name order.
func (c *ControlService) PluginDirectories(
	ctx context.Context,
	user webapi.UserID,
) (map[string][]plugin.HostDirectory, error) {
	return controlCall(
		ctx,
		c,
		func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) (map[string][]plugin.HostDirectory, error) {
			sets := make(map[string][]plugin.HostDirectory)
			_, err := queryRecords(
				ctx,
				tx,
				"plugin_directories",
				"SELECT plugin,name,files FROM plugin_directories WHERE user_id = ? ORDER BY plugin,name",
				func(r *storedRow) struct{} {
					id := r.text("plugin")
					sets[id] = append(
						sets[id],
						plugin.HostDirectory{Name: r.text("name"), Files: storedJSON(r, "files", decodeDirectoryFiles)},
					)
					return struct{}{}
				},
				user,
			)
			return sets, err
		},
	)
}

// SetPluginDirectories replaces `plugin`'s Host directories for `user` whole, in one
// transaction; the blobs the set named before and names now are used
// before it commits.
func (c *ControlService) SetPluginDirectories(
	ctx context.Context,
	user webapi.UserID,
	plugin string,
	directories []plugin.HostDirectory,
	uses OwnerBlobs,
) error {
	return controlDo(ctx, c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) error {
		touched, err := directoryBlobs(
			ctx,
			tx,
			"SELECT files FROM plugin_directories WHERE user_id = ? AND plugin = ?",
			user,
			plugin,
		)
		if err != nil {
			return err
		}
		if err := execSQL(
			ctx,
			tx,
			"DELETE FROM plugin_directories WHERE user_id = ? AND plugin = ?",
			user,
			plugin,
		); err != nil {
			return err
		}
		for _, directory := range directories {
			files, err := encoded(directoryFiles(directory.Files))
			if err != nil {
				return err
			}
			if err := execSQL(
				ctx,
				tx,
				"INSERT INTO plugin_directories (user_id,plugin,name,digest,files) VALUES (?,?,?,?,?)",
				user,
				plugin,
				directory.Name,
				directory.Digest(),
				files,
			); err != nil {
				return err
			}
			for _, file := range directory.Files {
				touched = append(touched, file.Blob)
			}
		}
		return uses.CommitUses(ctx, touched)
	})
}

// PluginBlobs returns every blob `user`'s plugin values and Host directories name, which
// the collector keeps.
func (c *ControlService) PluginBlobs(ctx context.Context, user webapi.UserID) ([]core.BlobRef, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) ([]core.BlobRef, error) {
		rows, err := queryRecords(
			ctx,
			tx,
			"plugin_values",
			"SELECT blobs FROM plugin_values WHERE user_id = ?",
			func(r *storedRow) blobNames { return storedJSON(r, "blobs", decodeBlobNames) },
			user,
		)
		if err != nil {
			return nil, err
		}
		blobs := make([]core.BlobRef, 0)
		for _, row := range rows {
			blobs = append(blobs, row...)
		}
		directories, err := directoryBlobs(ctx, tx, "SELECT files FROM plugin_directories WHERE user_id = ?", user)
		return append(blobs, directories...), err
	})
}

// The blobs a plugin value's column names.
// +demi:root
type blobNames []core.BlobRef

// A Host directory's stored files.
// +demi:root
type directoryFiles []plugin.DirectoryFile

type valueBlobs struct {
	revision uint64
	blobs    blobNames
}

func valueBlobRow(r *storedRow) valueBlobs {
	return valueBlobs{revision: r.count("revision"), blobs: storedJSON(r, "blobs", decodeBlobNames)}
}

func pluginValueRow(r *storedRow) PluginValue {
	document := json.RawMessage(r.text("document"))
	r.bad("document", contract.CheckJSON(document))
	return PluginValue{Document: document, Revision: r.count("revision")}
}

func directoryBlobs(ctx context.Context, tx *sql.Tx, query string, args ...any) ([]core.BlobRef, error) {
	rows, err := queryRecords(
		ctx,
		tx,
		"plugin_directories",
		query,
		func(r *storedRow) directoryFiles { return storedJSON(r, "files", decodeDirectoryFiles) },
		args...)
	if err != nil {
		return nil, err
	}
	blobs := make([]core.BlobRef, 0)
	for _, files := range rows {
		for _, file := range files {
			blobs = append(blobs, file.Blob)
		}
	}
	return blobs, nil
}
