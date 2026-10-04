//go:build linux

package machinemanager

import (
	"context"
	"net/url"

	"github.com/wspl/demi/internal/machinemanager/network"
	"github.com/wspl/demi/internal/machinemanager/sandbox"
	"github.com/wspl/demi/internal/machinemanager/storage"
	"github.com/wspl/demi/internal/machinemanager/system"
	"github.com/wspl/demi/internal/machinemanagerproto"
)

// Core holds immutable infrastructure shared by device workers.
type Core struct {
	// Config is the validated manager configuration.
	Config Config
	// Tools holds the resolved infrastructure executables.
	Tools *system.Tools
	// Runsc runs the pinned sandbox runtime.
	Runsc *sandbox.Runsc
	// Store owns committed image generations.
	Store *storage.Store
	// Network owns the host network policy.
	Network *network.Network
	// Slots owns network slot reservations.
	Slots *network.Pool
}

// NewCore assembles the infrastructure for validated configuration.
func NewCore(config Config, tools *system.Tools) (*Core, error) {
	backend, err := url.Parse(config.BackendURL.String())
	if err != nil {
		return nil, err
	}
	return &Core{
		Config:  config,
		Tools:   tools,
		Runsc:   sandbox.NewRunsc(tools, RuntimeDirectory, config.Limits != nil),
		Store:   storage.NewStore(config.Images()),
		Network: network.New(config.Subnet, config.DNS, *backend),
		Slots:   network.NewPool(config.Subnet, config.Slots),
	}, nil
}

func (c *Core) sandboxConfig() sandbox.Config {
	config := sandbox.Config{Runtime: RuntimeDirectory, BackendURL: c.Config.BackendURL, DNS: c.Config.DNS}
	if c.Config.Limits != nil {
		config.Limits = &sandbox.Limits{CPUs: c.Config.Limits.CPUs, MemoryMiB: c.Config.Limits.MemoryMiB}
	}
	return config
}

func (c *Core) dependencies() sandbox.Dependencies {
	return sandbox.Dependencies{Tools: c.Tools, Runsc: c.Runsc, Network: networkAdapter{c}, Disks: diskAdapter{c.Tools}}
}

// networkAdapter gives the sandbox its manager-selected network slot.
type networkAdapter struct{ core *Core }

// Attach connects the manager-selected network slot.
func (n networkAdapter) Attach(ctx context.Context, slot sandbox.Slot) error {
	return n.core.Network.Attach(ctx, n.core.Slots.Slot(slot.Index))
}

// Detach releases the manager-selected network slot.
func (n networkAdapter) Detach(ctx context.Context, slot sandbox.Slot) error {
	return n.core.Network.Detach(ctx, n.core.Slots.Slot(slot.Index))
}

// diskAdapter supplies storage operations to the sandbox boundary.
type diskAdapter struct{ tools *system.Tools }

// CloneSparse copies an image while preserving holes.
func (d diskAdapter) CloneSparse(ctx context.Context, source, destination string) error {
	return storage.CloneSparse(ctx, source, destination)
}

// GrowMounted grows the filesystem on the mounted device.
func (d diskAdapter) GrowMounted(ctx context.Context, device string) error {
	return storage.GrowMounted(ctx, d.tools, device)
}

// Capacity reads the filesystem capacity of the image.
func (d diskAdapter) Capacity(ctx context.Context, image string) (uint64, error) {
	return storage.Capacity(ctx, image)
}

// workingImages adapts the storage owner's volume lookup to the sandbox.
type workingImages struct{ *storage.WorkingPair }

// Image returns the path for the selected volume.
func (w workingImages) Image(v machinemanagerproto.Volume) string {
	return w.Images().ForVolume(v)
}

type imagePaths struct{ storage.ImagePair[string] }

// Image returns the path for the selected volume.
func (p imagePaths) Image(v machinemanagerproto.Volume) string {
	return p.ForVolume(v)
}
