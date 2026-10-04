package browser_test

import (
	"context"
	"encoding/json"
	"fmt"
	"testing"

	"github.com/wspl/demi/internal/declare"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/webapi"
)

type panelTransport func(context.Context, plugin.PortMessage) (plugin.PortAnswer, error)

func (p panelTransport) Request(ctx context.Context, m plugin.PortMessage) (plugin.PortAnswer, error) {
	return p(ctx, m)
}

// These scenarios use the plugin's JSON boundary and scripted Host and panel
// replies. No worker or wall-time wait; each scenario's budget is one second.
func TestCreatedTabFollowsLatestAddressOrClosesAfterRemoval(t *testing.T) {
	for _, removed := range []bool{false, true} {
		t.Run(fmt.Sprint("removed=", removed), func(t *testing.T) {
			var calls []string
			p, demi := world(
				t,
				func(
					_ context.Context,
					op declare.NativeOperation,
					args json.RawMessage,
					kind plugin.CallKind,
				) (json.RawMessage, error) {
					calls = append(calls, op.Operation+":"+string(kind)+":"+string(args))
					if op.Operation == "browser.open" {
						return json.RawMessage(`{"tab":"t1","url":"https://first.test"}`), nil
					}
					return json.RawMessage(`{}`), nil
				},
			)
			reads, updates := 0, 0
			demi.Panel = panelTransport(func(_ context.Context, m plugin.PortMessage) (plugin.PortAnswer, error) {
				switch m := any(m).(type) {
				case *plugin.PortMessagePanelTabs:
					reads++
					panel := webapi.EmptyWorkPanel()
					if reads == 1 {
						panel.Tabs = []webapi.PanelTab{
							{ID: "p1", Kind: "browser", Data: json.RawMessage(`{"url":"https://first.test"}`)},
						}
					}
					if reads == 2 && !removed {
						panel.Tabs = []webapi.PanelTab{
							{
								ID:   "p1",
								Kind: "browser",
								Data: json.RawMessage(`{"url":"https://last.test","tab":"t1"}`),
							},
						}
					}
					return &plugin.PortAnswerPanel{Panel: panel}, nil
				case *plugin.PortMessageUpdatePanelTab:
					updates++
					if m.ID != "p1" || string(m.Data) != `{"tab":"t1","closed":null,"failure":null}` {
						t.Errorf("patch got %s %s", m.ID, m.Data)
					}
					return &plugin.PortAnswerPanelRevision{Revision: 2}, nil
				default:
					return nil, fmt.Errorf("unexpected panel message %T", m)
				}
			})
			_, err := p.Call(
				t.Context(),
				&plugin.RequestPanelTab{
					User:         "u1",
					Conversation: "0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b",
					Change:       plugin.PanelTabCreated,
					Tab:          webapi.PanelTab{ID: "p1", Kind: "browser", Data: json.RawMessage(`{"url":"old"}`)},
				},
				demi.Port(),
			)
			if err != nil {
				t.Fatal(err)
			}
			want := `browser.goto:operates:{"tab":"t1","url":"https://last.test"}`
			if removed {
				want = `browser.close:operates:{"tab":"t1"}`
			}
			if len(calls) != 2 || calls[0] != `browser.open:starts:{"url":"https://first.test"}` || calls[1] != want ||
				updates != 1 {
				t.Fatalf("got calls %v updates %d; want open then %s and one patch", calls, updates, want)
			}
		})
	}
}

func TestFailedTabRecordsReasonAndRetryBinds(t *testing.T) {
	attempts, reads := 0, 0
	var patches []string
	p, demi := world(
		t,
		func(context.Context, declare.NativeOperation, json.RawMessage, plugin.CallKind) (json.RawMessage, error) {
			attempts++
			if attempts == 1 {
				return nil, &plugin.PortRefusalHost{
					Code:    webapi.ErrorCodeDeviceOffline,
					Status:  409,
					Message: "The device is offline",
				}
			}
			return json.RawMessage(`{"tab":"t2","url":"about:blank"}`), nil
		},
	)
	demi.Panel = panelTransport(func(_ context.Context, m plugin.PortMessage) (plugin.PortAnswer, error) {
		switch m := any(m).(type) {
		case *plugin.PortMessagePanelTabs:
			reads++
			data := `{"url":"about:blank"}`
			if reads == 2 {
				data = `{"url":"about:blank","failure":{"code":"device_offline","message":"The device is offline"}}`
			}
			if reads == 3 {
				data = `{"url":"about:blank","tab":"t2"}`
			}
			return &plugin.PortAnswerPanel{
				Panel: webapi.WorkPanel{
					Tabs: []webapi.PanelTab{{ID: "p1", Kind: "browser", Data: json.RawMessage(data)}},
				},
			}, nil
		case *plugin.PortMessageUpdatePanelTab:
			patches = append(patches, string(m.Data))
			return &plugin.PortAnswerPanelRevision{Revision: uint64(len(patches))}, nil
		default:
			return nil, fmt.Errorf("unexpected %T", m)
		}
	})
	for range 2 {
		if _, err := pageCall(t, p, demi, "bind", `{"panelTab":"p1"}`); err != nil {
			t.Fatal(err)
		}
	}
	if attempts != 2 || len(patches) != 2 ||
		patches[0] != `{"failure":{"code":"device_offline","message":"The device is offline"}}` ||
		patches[1] != `{"tab":"t2","closed":null,"failure":null}` {
		t.Fatalf("got attempts %d patches %v; want failure then binding", attempts, patches)
	}
}

func TestRemovedTabClosesItsBrowserTab(t *testing.T) {
	p, demi := world(
		t,
		func(
			_ context.Context,
			op declare.NativeOperation,
			args json.RawMessage,
			kind plugin.CallKind,
		) (json.RawMessage, error) {
			if op.Operation != "browser.close" || string(args) != `{"tab":"t9"}` || kind != plugin.CallKindOperates {
				t.Errorf("got %v %s %s; want close t9 without waking", op, args, kind)
			}
			return json.RawMessage(`{}`), nil
		},
	)
	_, err := p.Call(
		t.Context(),
		&plugin.RequestPanelTab{
			User:         "u1",
			Conversation: "0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b",
			Change:       plugin.PanelTabRemoved,
			Tab: webapi.PanelTab{
				ID:   "p1",
				Kind: "browser",
				Data: json.RawMessage(`{"url":"about:blank","tab":"t9","failure":null}`),
			},
		},
		demi.Port(),
	)
	if err != nil {
		t.Fatal(err)
	}
	if len(demi.Called()) != 1 {
		t.Fatalf("got %d calls; want 1", len(demi.Called()))
	}
}

func TestJobSyncUsesStableIDsAndMarksMissingTabsClosed(t *testing.T) {
	p, demi := world(
		t,
		func(context.Context, declare.NativeOperation, json.RawMessage, plugin.CallKind) (json.RawMessage, error) {
			return json.RawMessage(
				`{"tabs":[{"id":"t1","title":"A","url":"https://a.test","createdBy":{"kind":"user"}},` +
					`{"id":"t2","title":"B","url":"https://b.test","createdBy":{"kind":"agent","number":1}}],"truncated":false}`,
			), nil
		},
	)
	creates, updates := 0, 0
	demi.Panel = panelTransport(func(_ context.Context, m plugin.PortMessage) (plugin.PortAnswer, error) {
		switch m := any(m).(type) {
		case *plugin.PortMessagePanelTabs:
			return &plugin.PortAnswerPanel{
				Panel: webapi.WorkPanel{
					Tabs: []webapi.PanelTab{
						{ID: "p3", Kind: "browser", Data: json.RawMessage(`{"url":"gone","tab":"t3"}`)},
					},
				},
			}, nil
		case *plugin.PortMessageCreatePanelTab:
			creates++
			if m.Tab.ID != "browser-t2" || string(m.Tab.Data) != `{"url":"https://b.test","tab":"t2"}` {
				t.Errorf("got create %+v; want stable browser-t2", m.Tab)
			}
			return &plugin.PortAnswerPanelRevision{Revision: 1}, nil
		case *plugin.PortMessageUpdatePanelTab:
			updates++
			if m.ID != "p3" || string(m.Data) != `{"closed":true}` {
				t.Errorf("got update %+v; want closed p3", m)
			}
			return &plugin.PortAnswerPanelRevision{Revision: 2}, nil
		default:
			return nil, fmt.Errorf("unexpected %T", m)
		}
	})
	conversation := webapi.ConversationID("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b")
	for range 2 {
		_, err := p.Call(
			t.Context(),
			&plugin.RequestTopic{User: "u1", Topic: plugin.TopicJobs, Conversation: &conversation},
			demi.Port(),
		)
		if err != nil {
			t.Fatal(err)
		}
	}
	if creates != 2 || updates != 2 {
		t.Fatalf("got creates %d updates %d; want 2 each with identical IDs", creates, updates)
	}
}
