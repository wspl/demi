package database

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import (
	"context"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/webapi"
)

// UserPlugins returns each plugin `user` turned on or off, by id; one the user never
// switched is on.
func (c *ControlService) UserPlugins(ctx context.Context, user webapi.UserID) (map[string]bool, error) {
	panic("not written: b-database")
}

// SetUserPlugin records that `user` has `plugin` on or off.
func (c *ControlService) SetUserPlugin(ctx context.Context, user webapi.UserID, plugin string, enabled bool) error {
	panic("not written: b-database")
}

// PluginValue returns `plugin`'s value `key` for `user`.
func (c *ControlService) PluginValue(ctx context.Context, user webapi.UserID, plugin string, key string) (*PluginValue, error) {
	panic("not written: b-database")
}

// PluginValues returns every value of `plugin`'s for `user`, by key.
func (c *ControlService) PluginValues(ctx context.Context, user webapi.UserID, plugin string) (map[string]PluginValue, error) {
	panic("not written: b-database")
}

// WritePluginValue writes `plugin`'s value `key` for `user` if it is still at
// `revision`, none for a value that does not exist yet, in one
// transaction, so two writes never build on the same revision. The
// value names `blobs`; the blobs it named before and names now are
// used before it commits (`storage.md` § Collecting blobs).
func (c *ControlService) WritePluginValue(ctx context.Context, write ValueWrite, uses OwnerBlobs) (Written, error) {
	panic("not written: b-database")
}

// RemovePluginValue removes `plugin`'s value `key` for `user` if it is still at
// `revision`, in one transaction; the blobs it named are used before
// it commits. Answers WrittenRevision with the removed revision.
func (c *ControlService) RemovePluginValue(ctx context.Context, user webapi.UserID, plugin string, key string, revision uint64, uses OwnerBlobs) (Written, error) {
	panic("not written: b-database")
}

// PluginDirectories returns every Host directory of each of `user`'s plugins, by plugin id, each
// set in name order.
func (c *ControlService) PluginDirectories(ctx context.Context, user webapi.UserID) (map[string][]plugin.HostDirectory, error) {
	panic("not written: b-database")
}

// SetPluginDirectories replaces `plugin`'s Host directories for `user` whole, in one
// transaction; the blobs the set named before and names now are used
// before it commits.
func (c *ControlService) SetPluginDirectories(ctx context.Context, user webapi.UserID, plugin string, directories []plugin.HostDirectory, uses OwnerBlobs) error {
	panic("not written: b-database")
}

// PluginBlobs returns every blob `user`'s plugin values and Host directories name, which
// the collector keeps.
func (c *ControlService) PluginBlobs(ctx context.Context, user webapi.UserID) ([]core.BlobRef, error) {
	panic("not written: b-database")
}
