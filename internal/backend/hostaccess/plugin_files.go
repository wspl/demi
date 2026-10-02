package hostaccess

//revive:disable:unused-parameter

import (
	"context"
	"sync"

	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/webapi"
)

// DirectorySet is one plugin and its desired Host directories. An empty set
// removes the disabled plugin's directories at the next installation.
type DirectorySet struct {
	Plugin      plugin.ID
	Directories []plugin.HostDirectory
}

// DirectorySets is the ordered Host directory sets of the user's plugins.
type DirectorySets []DirectorySet

// PluginInstalls remembers each device's installed revision and connection.
// Its opaque state uses the shard mutex; installation waits occur outside it.
type PluginInstalls struct{}

// NewPluginInstalls creates the installation registry for the shard mutex.
func NewPluginInstalls(mu *sync.Mutex) *PluginInstalls { panic("not written: b-hostaccess") }

// ReadFiles reads the main Host as a look, with no activity or wake. A transition
// ends the read. Path failures answer unreadable; answers keep request order.
func ReadFiles(ctx context.Context, shard HostShard, id webapi.ConversationID, reads []plugin.HostRead) ([]plugin.HostFile, error) {
	panic("not written: b-hostaccess")
}
