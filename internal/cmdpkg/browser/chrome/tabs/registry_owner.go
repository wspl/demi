package tabs

import (
	"context"
	"errors"
	"log/slog"
	"time"

	protocol "github.com/chromedp/cdproto/cdp"
	"github.com/chromedp/cdproto/target"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
)

type registryKind uint8

const (
	registryCreate registryKind = iota
	registryCreated
	registryReady
	registryClose
	registryNotClosed
	registryHold
	registryRelease
	registryOpenedBy
	registryPopups
	registryRetitle
	registryTitled
	registryEvent
)

type registryReply struct {
	tab    *Tab
	ids    []browserop.TabID
	closed Closed
	err    error
}
type registryRequest struct {
	kind      registryKind
	ctx       context.Context
	target    target.ID
	createdBy browserop.BrowserCreatedBy
	tab       *Tab
	deadline  time.Time
	reply     chan registryReply
	err       error
	event     cdp.Event
	targets   []*target.Info
}
type registryOwner struct {
	environment *Environment
	book        *registryBook
	holds       int
	closing     map[target.ID]chan registryReply
	popups      []registryRequest
}

// ask sends one registry request; capacity-one replies never strand its owner.
func (e *Environment) ask(ctx context.Context, request registryRequest) (registryReply, error) {
	request.reply = make(chan registryReply, 1)
	request.ctx = ctx
	select {
	case e.requests <- request:
	case <-ctx.Done():
		return registryReply{}, ctx.Err()
	case <-e.ctx.Done():
		return registryReply{}, &cdp.BrowserError{Kind: cdp.KindClosed}
	}
	wait := ctx
	if request.kind == registryHold {
		// An admitted hold must be returned to its caller so cancellation cannot leak it.
		wait = context.WithoutCancel(ctx)
	}
	select {
	case reply := <-request.reply:
		return reply, reply.err
	case <-wait.Done():
		return registryReply{}, ctx.Err()
	case <-e.ctx.Done():
		select {
		case reply := <-request.reply:
			return reply, reply.err
		default:
			return registryReply{}, &cdp.BrowserError{Kind: cdp.KindClosed}
		}
	}
}

// create waits for cancellation cleanup as well as the admitted tab creation.
func (e *Environment) create(ctx context.Context, createdBy browserop.BrowserCreatedBy) (*Tab, error) {
	reply := make(chan registryReply, 1)
	request := registryRequest{kind: registryCreate, ctx: ctx, createdBy: createdBy, reply: reply}
	select {
	case e.requests <- request:
	case <-ctx.Done():
		return nil, ctx.Err()
	case <-e.ctx.Done():
		return nil, &cdp.BrowserError{Kind: cdp.KindClosed}
	}
	select {
	case result := <-reply:
		return result.tab, result.err
	case <-e.ctx.Done():
		return nil, &cdp.BrowserError{Kind: cdp.KindClosed}
	case <-ctx.Done():
		cleanup, cancel := context.WithTimeout(context.WithoutCancel(ctx), cdp.ControlTimeout)
		defer cancel()
		select {
		case result := <-reply:
			if result.tab != nil {
				return nil, cdp.AfterCleanup(ctx.Err(), result.tab.Close(cleanup, cdp.ControlTimeout))
			}
			return nil, cdp.AfterCleanup(ctx.Err(), result.err)
		case <-e.ctx.Done():
			return nil, ctx.Err()
		case <-cleanup.Done():
			return nil, cdp.AfterCleanup(ctx.Err(), cleanup.Err())
		}
	}
}

// tell returns registry task results only while the environment still has an owner.
func (e *Environment) tell(request registryRequest) {
	select {
	case e.requests <- request:
	case <-e.ctx.Done():
	}
}

// runRegistry owns tab bookkeeping; setup and browser requests run in joined workers.
func (e *Environment) runRegistry(ctx context.Context, events *cdp.Subscription) {
	defer events.Close()
	eventCtx, cancelEvents := context.WithCancel(ctx)
	pumpDone := make(chan struct{})
	go func() {
		defer close(pumpDone)
		for {
			event, err := events.Next(eventCtx)
			if eventCtx.Err() != nil {
				return
			}
			select {
			case e.requests <- registryRequest{kind: registryEvent, event: event, err: err}:
			case <-eventCtx.Done():
				return
			}
			if err != nil {
				var lost *cdp.EventLoss
				if !errors.As(err, &lost) {
					return
				}
			}
		}
	}()
	owner := &registryOwner{environment: e, book: newRegistryBook(), closing: make(map[target.ID]chan registryReply)}
	defer func() {
		cancelEvents()
		<-pumpDone
		for _, entry := range owner.book.entries {
			if entry.tab != nil {
				cleanup, cancel := context.WithTimeout(context.Background(), cdp.ControlTimeout)
				if err := entry.tab.shutdown(cleanup); err != nil {
					slog.Warn("tab cleanup failed", "error", err)
				}
				cancel()
			}
		}
	}()
	for {
		select {
		case <-ctx.Done():
			return
		case request := <-e.requests:
			owner.request(ctx, request)
			pending := owner.popups[:0]
			for _, query := range owner.popups {
				if query.ctx.Err() != nil {
					continue
				}
				ids, ready := owner.book.popups(query.target)
				if ready {
					query.reply <- registryReply{ids: ids}
				} else {
					pending = append(pending, query)
				}
			}
			owner.popups = pending
		}
	}
}

// number names a Chrome page once, retaining its ID even after closure.
func (r *registryOwner) number(ctx context.Context, id target.ID) (browserop.TabID, error) {
	if known, ok := r.book.publicIDs[id]; ok {
		return known.id, nil
	}
	if r.environment.numbers == nil {
		return "", &cdp.BrowserError{Kind: cdp.KindUnavailable, Message: "the service was given no tab numbers"}
	}
	number, err := r.environment.numbers.Next(ctx)
	if err != nil {
		return "", err
	}
	return r.book.name(id, number), nil
}

func (r *registryOwner) publish() {
	tabs, registering := r.book.listing()
	changed := make(chan struct{})
	old := r.environment.snapshotChanged
	r.environment.snapshotChanged = changed
	r.environment.snapshot.Store(&Snapshot{Tabs: tabs, Registering: registering, Changed: changed})
	close(old)
	r.environment.markChanged()
}

func (r *registryOwner) settle() {
	if r.book.settle(r.holds) {
		close(r.environment.emptied)
	}
}

// found adopts site-created top-level pages, keeping Chrome background targets out.
func (r *registryOwner) found(ctx context.Context, info *target.Info) {
	if info.Type != "page" {
		return
	}
	changed := r.book.found(info)
	if changed {
		r.publish()
	}
	if !r.book.pending(info.TargetID) || r.book.openers[info.TargetID] == "" {
		return
	}
	id, err := r.number(ctx, info.TargetID)
	if err != nil {
		r.refuse(info.TargetID, err)
		return
	}
	opener, err := r.number(ctx, r.book.openers[info.TargetID])
	if err != nil {
		r.refuse(info.TargetID, err)
		return
	}
	r.setup(ctx, info.TargetID, id, &browserop.BrowserCreatedByPage{Opener: opener}, nil)
}

func (r *registryOwner) refuse(id target.ID, err error) {
	slog.Warn("a popup is closed without a tab number", "error", err)
	r.book.unusable(id)
	r.publish()
	r.settle()
	r.closePage(id)
}

func (r *registryOwner) closePage(id target.ID) {
	e := r.environment
	if err := e.StartTask(func(ctx context.Context) {
		bounded, cancel := context.WithTimeout(ctx, cdp.ControlTimeout)
		defer cancel()
		if err := target.CloseTarget(id).Do(protocol.WithExecutor(bounded, e.connection)); err != nil && ctx.Err() == nil {
			slog.Warn("could not close a browser page that is not a tab", "error", err)
		}
	}); err != nil {
		return
	} // Environment retirement owns every remaining page.
}

func (r *registryOwner) setup(requestCtx context.Context, id target.ID, public browserop.TabID, createdBy browserop.BrowserCreatedBy, reply chan registryReply) {
	r.book.setUp(id)
	r.publish()
	e := r.environment
	if err := e.StartTask(func(ctx context.Context) {
		bounded, cancel := context.WithTimeout(ctx, 30*time.Second)
		defer cancel()
		tab, err := e.setUpTab(bounded, id, public, createdBy)
		e.tell(registryRequest{kind: registryReady, target: id, tab: tab, err: err, reply: reply, ctx: requestCtx})
	}); err != nil && reply != nil {
		reply <- registryReply{err: err}
	}
}

func (r *registryOwner) gone(id target.ID) {
	tab, changed := r.book.gone(id)
	reply := r.closing[id]
	delete(r.closing, id)
	if tab != nil {
		tab.cancel(&cdp.BrowserError{Kind: cdp.KindClosed})
		cleanup := func(ctx context.Context) {
			bounded, cancel := context.WithTimeout(context.WithoutCancel(ctx), cdp.ControlTimeout)
			defer cancel()
			err := tab.shutdown(bounded)
			if reply != nil {
				reply <- registryReply{closed: ClosedTab, err: err}
			} else if err != nil {
				slog.Warn("closed tab cleanup failed", "error", err)
			}
		}
		if err := r.environment.StartTask(cleanup); err != nil {
			// Retirement has stopped admission; the registry still owns this tab.
			cleanup(r.environment.ctx)
		}
	} else if reply != nil {
		reply <- registryReply{closed: ClosedTab}
	}
	if changed {
		r.publish()
	}
	r.settle()
}

// reconcile pays for dropped target events with a current Chrome target list.
func (r *registryOwner) reconcile(ctx context.Context) {
	r.book.reconciling = true
	r.publish()
	targets, err := target.GetTargets().Do(protocol.WithExecutor(ctx, r.environment.Browser()))
	if err == nil {
		present := make(map[target.ID]bool)
		for _, info := range targets {
			if info.Type == "page" {
				present[info.TargetID] = true
				r.found(ctx, info)
			}
		}
		for _, id := range r.book.vanished(present) {
			r.gone(id)
		}
	} else {
		slog.Debug("the browser tab registry could not reconcile", "error", err)
	}
	r.book.reconciling = false
	r.publish()
}

func (r *registryOwner) request(ctx context.Context, q registryRequest) {
	e := r.environment
	switch q.kind {
	case registryCreate:
		if !r.book.admit() {
			q.reply <- registryReply{err: &cdp.BrowserError{Kind: cdp.KindClosed}}
			return
		}
		if err := e.StartTask(func(ctx context.Context) {
			// cdproto's CreateTargetParams always writes optional booleans as false.
			// Chrome distinguishes omitted newWindow from false when no window exists;
			// chromiumoxide sends only url. Use that vendor payload and its typed reply.
			params := struct {
				URL string `json:"url"`
			}{URL: "about:blank"}
			var result target.CreateTargetReturns
			err := e.Browser().Execute(ctx, target.CommandCreateTarget, params, &result)
			q.kind = registryCreated
			q.target = result.TargetID
			q.err = err
			e.tell(q)
		}); err != nil {
			r.book.failed()
			q.reply <- registryReply{err: err}
			r.settle()
		}
	case registryCreated:
		if q.err == nil {
			public, err := r.number(ctx, q.target)
			q.err = err
			if err == nil {
				r.setup(q.ctx, q.target, public, q.createdBy, q.reply)
				return
			}
			r.closePage(q.target)
		}
		r.book.failed()
		r.settle()
		q.reply <- registryReply{err: q.err}
	case registryReady:
		refused := r.book.ready(q.target, q.reply != nil, q.tab)
		err := q.err
		if err == nil {
			err = refused
		}
		r.publish()
		r.settle()
		if q.tab != nil && (err != nil || q.ctx != nil && q.ctx.Err() != nil) {
			if err == nil {
				err = q.ctx.Err()
			}
			q.tab.cancel(&cdp.BrowserError{Kind: cdp.KindClosed})
			cleanupErr := err
			if startErr := e.StartTask(func(ctx context.Context) {
				bounded, cancel := context.WithTimeout(ctx, cdp.ControlTimeout)
				defer cancel()
				closed := q.tab.closeTarget(bounded)
				if q.reply != nil {
					q.reply <- registryReply{err: cdp.AfterCleanup(cleanupErr, closed)}
				}
			}); startErr != nil && q.reply != nil {
				q.reply <- registryReply{err: startErr}
			}
			return
		}
		if q.reply != nil {
			q.reply <- registryReply{tab: q.tab, err: err}
		}
		if err != nil {
			r.closePage(q.target)
		}
	case registryClose:
		if r.book.sealed {
			q.reply <- registryReply{err: &cdp.BrowserError{Kind: cdp.KindClosed}}
			return
		}
		if r.book.closable(q.target) && r.book.only(q.target, r.holds) {
			r.reconcile(ctx)
			if r.book.closable(q.target) && r.book.only(q.target, r.holds) {
				r.book.sealed = true
				close(e.emptied)
				q.reply <- registryReply{closed: ClosedEnvironment}
				return
			}
		}
		tab := r.book.startClosing(q.target)
		if tab == nil {
			q.reply <- registryReply{err: &cdp.BrowserError{Kind: cdp.KindTabNotFound}}
			return
		}
		r.closing[q.target] = q.reply
		tab.cancel(&cdp.BrowserError{Kind: cdp.KindClosed})
		if err := e.StartTask(func(ctx context.Context) {
			bounded, cancel := context.WithDeadline(ctx, q.deadline)
			defer cancel()
			if err := tab.closeTarget(bounded); err != nil {
				e.tell(registryRequest{kind: registryNotClosed, target: q.target, err: err})
			}
		}); err != nil {
			q.reply <- registryReply{err: err}
			delete(r.closing, q.target)
		}
	case registryNotClosed:
		r.book.notClosed(q.target)
		if reply := r.closing[q.target]; reply != nil {
			reply <- registryReply{err: q.err}
			delete(r.closing, q.target)
		}
	case registryHold:
		if err := q.ctx.Err(); err != nil {
			q.reply <- registryReply{err: err}
			return
		}
		if r.book.sealed {
			q.reply <- registryReply{err: &cdp.BrowserError{Kind: cdp.KindClosed}}
			return
		}
		r.holds++
		q.reply <- registryReply{}
	case registryRelease:
		r.holds--
		r.settle()
		q.reply <- registryReply{}
	case registryOpenedBy:
		q.reply <- registryReply{ids: r.book.opened(q.target)}
	case registryPopups:
		r.popups = append(r.popups, q)
	case registryRetitle:
		if err := e.StartTask(func(ctx context.Context) {
			targets, err := target.GetTargets().Do(protocol.WithExecutor(ctx, e.Browser()))
			e.tell(registryRequest{kind: registryTitled, targets: targets, err: err, reply: q.reply})
		}); err != nil {
			q.reply <- registryReply{err: err}
		}
	case registryTitled:
		changed := false
		if q.err == nil {
			for _, info := range q.targets {
				changed = r.book.retitle(info) || changed
			}
		}
		if changed {
			r.publish()
		}
		q.reply <- registryReply{err: q.err}
	case registryEvent:
		if q.err != nil {
			var lost *cdp.EventLoss
			if errors.As(q.err, &lost) {
				r.reconcile(ctx)
			}
			return
		}
		event, err := cdp.DecodeEvent(q.event)
		if err != nil {
			e.cancel(err)
			return
		}
		switch event := event.(type) {
		case *target.EventTargetCreated:
			r.found(ctx, event.TargetInfo)
		case *target.EventTargetInfoChanged:
			r.found(ctx, event.TargetInfo)
		case *target.EventTargetDestroyed:
			r.gone(event.TargetID)
		}
	}
}
