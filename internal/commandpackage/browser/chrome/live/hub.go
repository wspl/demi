package live

import (
	"context"
	"log/slog"
	"sync"

	"github.com/wspl/demi/internal/commandpackage/browser/browserproto"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/cdp"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/tabs"
)

// Hub is the way to one browser's live view hub. Viewers of a tab share its
// capture and encoding; the hub owns their screen and viewport decisions and
// each watched tab's page observer. It does not own the environment.
// A Hub must be created with Start and must not be copied.
type Hub struct {
	environment *tabs.Environment
	requests    chan func(context.Context, *hubOwner)
	wake        chan struct{}
	// mu protects observer initialization entries, never browser calls.
	mu        sync.Mutex
	observers map[*tabs.Tab]*observerEntry
}

// Start starts the hub of environment; it ends with the environment.
// The browser owner starts exactly one hub per environment. All hub workers
// register with environment.StartTask, and environment.Close cancels and joins
// them. Start returns an error if the environment has stopped admitting work.
func Start(environment *tabs.Environment) (*Hub, error) {
	h := &Hub{
		environment: environment,
		requests:    make(chan func(context.Context, *hubOwner), 32),
		wake:        make(chan struct{}, 1),
		observers:   make(map[*tabs.Tab]*observerEntry),
	}
	err := environment.StartTask(h.run)
	if err != nil {
		return nil, err
	}
	return h, nil
}

type panel struct {
	width, height, screenWidth, screenHeight uint32
	ratio                                    float64
}
type (
	notice     struct{ code, message string }
	membership struct {
		hub     *Hub
		left    context.Context
		leave   context.CancelFunc
		notices chan notice
		id      uint64
	}
)

type hubViewer struct {
	member   *membership
	panel    *panel
	tab      *tabs.Tab
	operated uint64
}
type hubOwner struct {
	hub           *Hub
	viewers       map[uint64]*hubViewer
	streams       map[browserproto.TabID]*captureStream
	order, driver uint64
	screen        *tabs.Screen
	due           bool
}

func (h *Hub) run(ctx context.Context) {
	o := hubOwner{hub: h, viewers: make(map[uint64]*hubViewer), streams: make(map[browserproto.TabID]*captureStream)}
	defer func() {
		for _, s := range o.streams {
			s.close(ctx)
		}
	}()
	for {
		select {
		case <-ctx.Done():
			return
		case request := <-h.requests:
			request(ctx, &o)
		case <-h.wake:
		}
		draining := true
		for draining {
			select {
			case request := <-h.requests:
				request(ctx, &o)
			default:
				draining = false
			}
		}
		for id, v := range o.viewers {
			if v.member.left.Err() != nil {
				o.leave(ctx, id)
			}
		}
		if o.due && ctx.Err() == nil {
			o.due = false
			o.layout(ctx)
		}
	}
}

func (h *Hub) send(ctx context.Context, request func(context.Context, *hubOwner)) error {
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-h.environment.Done():
		return &cdp.BrowserError{Kind: cdp.KindClosed}
	case h.requests <- request:
		return nil
	}
}

func (h *Hub) join(ctx context.Context) (*membership, error) {
	left, leave := context.WithCancel(ctx)
	m := &membership{hub: h, left: left, leave: leave, notices: make(chan notice, 4)}
	reply := make(chan struct{}, 1)
	if err := h.send(ctx, func(_ context.Context, o *hubOwner) {
		o.order++
		m.id = o.order
		o.viewers[m.id] = &hubViewer{member: m}
		reply <- struct{}{}
	}); err != nil {
		leave()
		return nil, err
	}
	select {
	case <-reply:
		return m, nil
	case <-ctx.Done():
		m.close()
		return nil, ctx.Err()
	case <-h.environment.Done():
		m.close()
		return nil, &cdp.BrowserError{Kind: cdp.KindClosed}
	}
}

func (m *membership) close() {
	m.leave()
	select {
	case m.hub.wake <- struct{}{}:
	default:
	}
}

func (m *membership) setPanel(ctx context.Context, p panel) error {
	return m.hub.send(ctx, func(_ context.Context, o *hubOwner) {
		v := o.viewers[m.id]
		if v == nil {
			return
		}
		v.panel = &p
		if o.viewers[o.driver] == nil {
			o.driver = m.id
		}
		o.due = true
	})
}

func (m *membership) operated(ctx context.Context) error {
	return m.hub.send(ctx, func(_ context.Context, o *hubOwner) {
		o.order++
		v := o.viewers[m.id]
		if v == nil {
			return
		}
		if o.driver != m.id || (v.operated == 0 && len(o.viewers) > 1) {
			o.due = true
		}
		v.operated = o.order
		o.driver = m.id
	})
}

func (m *membership) watch(ctx context.Context, tab *tabs.Tab) (*streamView, error) {
	type answer struct {
		view *streamView
		err  error
	}
	reply := make(chan answer, 1)
	err := m.hub.send(ctx, func(ownerCtx context.Context, o *hubOwner) {
		v := o.viewers[m.id]
		if v == nil || m.left.Err() != nil {
			reply <- answer{}
			return
		}
		if v.tab != nil {
			o.leaveStream(ownerCtx, v.tab.ID(), m.id)
		}
		v.tab = tab
		o.due = true
		if tab == nil {
			reply <- answer{}
			return
		}
		s := o.streams[tab.ID()]
		if s == nil {
			var err error
			s, err = startStream(ownerCtx, tab, o.hub.environment.Captures())
			if err != nil {
				reply <- answer{err: err}
				return
			}
			o.streams[tab.ID()] = s
		}
		reply <- answer{view: s.join(m.id)}
	})
	if err != nil {
		return nil, err
	}
	select {
	case a := <-reply:
		return a.view, a.err
	case <-ctx.Done():
		return nil, ctx.Err()
	case <-m.hub.environment.Done():
		return nil, &cdp.BrowserError{Kind: cdp.KindClosed}
	}
}

func (m *membership) mode(ctx context.Context, tab *tabs.Tab, mode browserproto.ViewportMode) error {
	reply := make(chan error, 1)
	err := m.hub.send(ctx, func(ownerCtx context.Context, o *hubOwner) {
		p := o.decider(tab.ID())
		if p == nil {
			if v := o.viewers[m.id]; v != nil {
				p = v.panel
			}
		}
		if p != nil {
			reply <- fit(ownerCtx, tab, mode, *p)
			return
		}
		web := tab.WebViewport()
		width, height := web.Width, web.Height
		if mode == "mobile" {
			width, height = tabs.PhoneWidth, tabs.PhoneHeight
		}
		reply <- tab.SetViewport(
			ownerCtx,
			browserproto.BrowserViewport{
				Mode:             mode,
				Width:            width,
				Height:           height,
				DevicePixelRatio: web.DevicePixelRatio,
			},
		)
	})
	if err != nil {
		return err
	}
	select {
	case err := <-reply:
		return err
	case <-ctx.Done():
		return ctx.Err()
	case <-m.hub.environment.Done():
		return &cdp.BrowserError{Kind: cdp.KindClosed}
	}
}

func (o *hubOwner) leave(ctx context.Context, id uint64) {
	if v := o.viewers[id]; v != nil && v.tab != nil {
		o.leaveStream(ctx, v.tab.ID(), id)
	}
	delete(o.viewers, id)
	if o.driver == id {
		o.driver = 0
	}
	o.due = true
}

func (o *hubOwner) leaveStream(ctx context.Context, tab browserproto.TabID, id uint64) {
	if s := o.streams[tab]; s != nil && s.leave(id) {
		s.close(ctx)
		delete(o.streams, tab)
	}
}

func (o *hubOwner) decider(tab browserproto.TabID) *panel {
	var chosen *hubViewer
	for _, v := range o.viewers {
		if v.tab == nil || v.tab.ID() != tab || v.panel == nil {
			continue
		}
		if chosen == nil || v.operated > chosen.operated ||
			(v.operated == chosen.operated && v.member.id < chosen.member.id) {
			chosen = v
		}
	}
	if chosen == nil {
		return nil
	}
	return chosen.panel
}

func (o *hubOwner) notify(chosen func(uint64, *hubViewer) bool, err error) {
	slog.Warn("live view layout", "error", err)
	for id, v := range o.viewers {
		if chosen(id, v) {
			select {
			case v.member.notices <- notice{string(cdp.ErrorCode(err)), err.Error()}:
			default:
			}
		}
	}
}

func (o *hubOwner) layout(ctx context.Context) {
	watched := make(map[browserproto.TabID]*tabs.Tab)
	for _, v := range o.viewers {
		if v.tab != nil && o.decider(v.tab.ID()) != nil {
			watched[v.tab.ID()] = v.tab
		}
	}
	if driver := o.viewers[o.driver]; driver != nil && driver.panel != nil {
		p := driver.panel
		desired := tabs.Screen{Width: p.screenWidth, Height: p.screenHeight, Ratio: tabs.ScreenRatio(p.ratio)}
		if o.screen == nil || *o.screen != desired {
			for _, tab := range watched {
				if err := tab.UpdateScreen(ctx, desired); err != nil {
					o.notify(func(id uint64, _ *hubViewer) bool {
						return id == o.driver
					}, err)
				} else {
					o.screen = &desired
				}
				break
			}
		}
	}
	for id, tab := range watched {
		if err := fit(ctx, tab, tab.Viewport().Mode, *o.decider(id)); err != nil && tab.Context().Err() == nil {
			o.notify(func(_ uint64, v *hubViewer) bool {
				return v.tab != nil && v.tab.ID() == id
			}, err)
		}
	}
}

func fit(ctx context.Context, tab *tabs.Tab, mode browserproto.ViewportMode, p panel) error {
	width, height := p.width, p.height
	switch mode {
	case "custom":
		return nil
	case "mobile":
		width, height = tabs.PhoneWidth, tabs.PhoneHeight
	}
	viewport := browserproto.BrowserViewport{
		Mode:             mode,
		Width:            width,
		Height:           height,
		DevicePixelRatio: tabs.RatioFor(p.ratio, width, height),
	}
	if tab.Viewport() == viewport {
		return nil
	}
	return tab.SetViewport(ctx, viewport)
}
