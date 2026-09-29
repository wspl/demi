package network

import (
	"encoding/json/v2"
	"net/netip"
)

// The firewall (docs/cloud/managed-hosts.md § Networking): one fixed nftables
// table whose rules are the same for every sandbox. Its only per-sandbox fact is
// an element of the slots set, the pair of a sandbox's host interface and source
// address, so a packet from a Cloud interface with any other pair is dropped.
//
//	table inet demi_cloud {
//	  set slots { type ifname . ipv4_addr; }
//	  chain input   { filter hook input -10; iifname "demih*" jump ingress }
//	  chain forward { filter hook forward -10; iifname "demih*" jump ingress;
//	                  oifname "demih*" ct state established,related accept; oifname "demih*" drop }
//	  chain ingress { meta nfproto != ipv4 drop; iifname . ip saddr != @slots drop;
//	                  ip daddr {backend} tcp dport <port> accept;
//	                  ip daddr {dns} meta l4proto {tcp, udp} th dport 53 accept;
//	                  fib daddr type local drop; ip daddr {denied} drop; accept }
//	  chain nat     { nat hook postrouting srcnat; ip saddr <pool> oifname != "demih*" masquerade }
//	}
//
// The rules are built as Go types for the subset of libnftables-json(5) the table
// uses and applied with nft in one transaction.

// The table, the set and the priorities of the chains.
const (
	// Table is the name of the table.
	Table = "demi_cloud"
	set   = "slots"
	// interfaces matches every Cloud host interface's name.
	interfaces = "demih*"
	// filterPriority runs the filter chains just before the default priority.
	filterPriority = -10
	// sourceNATPriority is srcnat: the priority of source NAT in the postrouting
	// hook.
	sourceNATPriority = 100
)

// denied are the destinations a sandbox may not reach unless an exact rule before
// this one allows it: private, shared, loopback, link-local (metadata),
// documentation, benchmarking, multicast and reserved ranges.
var denied = [...]netip.Prefix{
	netip.MustParsePrefix("0.0.0.0/8"),
	netip.MustParsePrefix("10.0.0.0/8"),
	netip.MustParsePrefix("100.64.0.0/10"),
	netip.MustParsePrefix("127.0.0.0/8"),
	netip.MustParsePrefix("169.254.0.0/16"),
	netip.MustParsePrefix("172.16.0.0/12"),
	netip.MustParsePrefix("192.0.0.0/24"),
	netip.MustParsePrefix("192.0.2.0/24"),
	netip.MustParsePrefix("192.168.0.0/16"),
	netip.MustParsePrefix("198.18.0.0/15"),
	netip.MustParsePrefix("198.51.100.0/24"),
	netip.MustParsePrefix("203.0.113.0/24"),
	netip.MustParsePrefix("224.0.0.0/4"),
	netip.MustParsePrefix("240.0.0.0/4"),
}

// A Policy is the allowed endpoints besides public destinations.
type Policy struct {
	Pool        netip.Prefix
	Backend     []netip.Addr
	BackendPort uint16
	DNS         []netip.Addr
}

// A Transaction is the nftables JSON nft applies in one transaction
// (libnftables-json(5)).
type Transaction struct {
	Nftables []command `json:"nftables"`
}

// Encode returns the transaction's JSON.
func (t Transaction) Encode() ([]byte, error) {
	return json.Marshal(t)
}

// A command adds or deletes an object.
type command struct {
	Add    *object `json:"add,omitzero"`
	Delete *object `json:"delete,omitzero"`
}

// An object is a table, set, chain, rule or element.
type object struct {
	Table   *tableObject   `json:"table,omitzero"`
	Set     *setObject     `json:"set,omitzero"`
	Chain   *chainObject   `json:"chain,omitzero"`
	Rule    *ruleObject    `json:"rule,omitzero"`
	Element *elementObject `json:"element,omitzero"`
}

type tableObject struct {
	Family string `json:"family"`
	Name   string `json:"name"`
}

type setObject struct {
	Family string   `json:"family"`
	Table  string   `json:"table"`
	Name   string   `json:"name"`
	Type   []string `json:"type"`
}

type chainObject struct {
	Family string `json:"family"`
	Table  string `json:"table"`
	Name   string `json:"name"`
	Type   string `json:"type,omitzero"`
	Hook   string `json:"hook,omitzero"`
	Prio   *int   `json:"prio,omitzero"`
	Policy string `json:"policy,omitzero"`
}

type ruleObject struct {
	Family string      `json:"family"`
	Table  string      `json:"table"`
	Chain  string      `json:"chain"`
	Expr   []statement `json:"expr"`
}

type elementObject struct {
	Family string       `json:"family"`
	Table  string       `json:"table"`
	Name   string       `json:"name"`
	Elem   []expression `json:"elem"`
}

// A statement is one step of a rule: a match, a jump or a verdict.
type statement any

// An expression is a value a match compares: a string or a number, or one of the
// objects below.
type expression any

type matchStatement struct {
	Match struct {
		Op    string     `json:"op"`
		Left  expression `json:"left"`
		Right expression `json:"right"`
	} `json:"match"`
}

type jumpStatement struct {
	Jump struct {
		Target string `json:"target"`
	} `json:"jump"`
}

// A verdict is a statement with no operands; its JSON is {"accept": null} and so
// on.
type acceptStatement struct {
	Accept *struct{} `json:"accept"`
}

type dropStatement struct {
	Drop *struct{} `json:"drop"`
}

type masqueradeStatement struct {
	Masquerade *struct{} `json:"masquerade"`
}

type metaExpression struct {
	Meta struct {
		Key string `json:"key"`
	} `json:"meta"`
}

type payloadExpression struct {
	Payload struct {
		Protocol string `json:"protocol"`
		Field    string `json:"field"`
	} `json:"payload"`
}

type concatExpression struct {
	Concat []expression `json:"concat"`
}

type setExpression struct {
	Set []expression `json:"set"`
}

type prefixExpression struct {
	Prefix struct {
		Addr string `json:"addr"`
		Len  int    `json:"len"`
	} `json:"prefix"`
}

type ctExpression struct {
	CT struct {
		Key string `json:"key"`
	} `json:"ct"`
}

type fibExpression struct {
	Fib struct {
		Result string   `json:"result"`
		Flags  []string `json:"flags"`
	} `json:"fib"`
}

// The operators of a match.
const (
	equal    = "=="
	notEqual = "!="
	within   = "in"
)

func matches(left, right expression) statement {
	return compare(equal, left, right)
}

func differs(left, right expression) statement {
	return compare(notEqual, left, right)
}

func compare(op string, left, right expression) statement {
	var statement matchStatement
	statement.Match.Op = op
	statement.Match.Left = left
	statement.Match.Right = right
	return statement
}

func jumpTo(chain string) statement {
	var statement jumpStatement
	statement.Jump.Target = chain
	return statement
}

func metaOf(key string) expression {
	var expression metaExpression
	expression.Meta.Key = key
	return expression
}

func payloadOf(protocol, field string) expression {
	var expression payloadExpression
	expression.Payload.Protocol = protocol
	expression.Payload.Field = field
	return expression
}

func setOfStrings(values ...string) expression {
	items := make([]expression, len(values))
	for i, value := range values {
		items[i] = value
	}
	return setExpression{Set: items}
}

func addresses(addresses []netip.Addr) expression {
	values := make([]string, len(addresses))
	for i, address := range addresses {
		values[i] = address.String()
	}
	return setOfStrings(values...)
}

func prefixOf(prefix netip.Prefix) expression {
	var expression prefixExpression
	expression.Prefix.Addr = prefix.Addr().String()
	expression.Prefix.Len = prefix.Bits()
	return expression
}

func tableObjectOf() *tableObject {
	return &tableObject{Family: "inet", Name: Table}
}

func setObjectOf() *setObject {
	return &setObject{Family: "inet", Table: Table, Name: set, Type: []string{"ifname", "ipv4_addr"}}
}

func elementObjectOf(slot Slot) *elementObject {
	return &elementObject{
		Family: "inet",
		Table:  Table,
		Name:   set,
		Elem:   []expression{concatExpression{Concat: []expression{slot.HostInterface(), slot.Address.String()}}},
	}
}

func baseChain(name, kind, hook string, priority int) *chainObject {
	return &chainObject{Family: "inet", Table: Table, Name: name, Type: kind, Hook: hook, Prio: &priority, Policy: "accept"}
}

func regularChain(name string) *chainObject {
	return &chainObject{Family: "inet", Table: Table, Name: name}
}

// TableTransaction returns the whole table, replacing any table of that name in
// one transaction: adding it first makes the deletion succeed whether or not it
// existed.
func TableTransaction(policy Policy) Transaction {
	var t Transaction
	add := func(o object) { t.Nftables = append(t.Nftables, command{Add: &o}) }
	t.Nftables = append(t.Nftables, command{Add: &object{Table: tableObjectOf()}})
	t.Nftables = append(t.Nftables, command{Delete: &object{Table: tableObjectOf()}})
	add(object{Table: tableObjectOf()})
	add(object{Set: setObjectOf()})
	add(object{Chain: baseChain("input", "filter", "input", filterPriority)})
	add(object{Chain: baseChain("forward", "filter", "forward", filterPriority)})
	add(object{Chain: regularChain("ingress")})
	add(object{Chain: baseChain("nat", "nat", "postrouting", sourceNATPriority)})
	var state ctExpression
	state.CT.Key = "state"
	var iifSaddr concatExpression
	iifSaddr.Concat = []expression{metaOf("iifname"), payloadOf("ip", "saddr")}
	deniedSet := make([]expression, len(denied))
	for i, prefix := range denied {
		deniedSet[i] = prefixOf(prefix)
	}
	rules := []struct {
		chain      string
		statements []statement
	}{
		{"input", []statement{matches(metaOf("iifname"), interfaces), jumpTo("ingress")}},
		{"forward", []statement{matches(metaOf("iifname"), interfaces), jumpTo("ingress")}},
		{"forward", []statement{
			matches(metaOf("oifname"), interfaces),
			compare(within, state, []expression{"established", "related"}),
			acceptStatement{},
		}},
		{"forward", []statement{matches(metaOf("oifname"), interfaces), dropStatement{}}},
		{"ingress", []statement{differs(metaOf("nfproto"), "ipv4"), dropStatement{}}},
		{"ingress", []statement{differs(iifSaddr, "@"+set), dropStatement{}}},
		{"ingress", []statement{
			matches(payloadOf("ip", "daddr"), addresses(policy.Backend)),
			matches(payloadOf("tcp", "dport"), uint32(policy.BackendPort)),
			acceptStatement{},
		}},
		{"ingress", []statement{
			matches(payloadOf("ip", "daddr"), addresses(policy.DNS)),
			matches(metaOf("l4proto"), setOfStrings("tcp", "udp")),
			matches(payloadOf("th", "dport"), uint32(53)),
			acceptStatement{},
		}},
		{"ingress", []statement{matches(destinationType(), "local"), dropStatement{}}},
		{"ingress", []statement{matches(payloadOf("ip", "daddr"), setExpression{Set: deniedSet}), dropStatement{}}},
		{"ingress", []statement{acceptStatement{}}},
		{"nat", []statement{
			matches(payloadOf("ip", "saddr"), prefixOf(policy.Pool.Masked())),
			differs(metaOf("oifname"), interfaces),
			masqueradeStatement{},
		}},
	}
	for _, rule := range rules {
		add(object{Rule: &ruleObject{Family: "inet", Table: Table, Chain: rule.chain, Expr: rule.statements}})
	}
	return t
}

// destinationType is the type of the packet's destination address, "local" for the
// host's own.
func destinationType() expression {
	var expression fibExpression
	expression.Fib.Result = "type"
	expression.Fib.Flags = []string{"daddr"}
	return expression
}

// AddSlotTransaction admits slot's pair before its sandbox runs.
func AddSlotTransaction(slot Slot) Transaction {
	return Transaction{Nftables: []command{{Add: &object{Element: elementObjectOf(slot)}}}}
}

// RemoveSlotTransaction removes slot's pair if the table has it. Adding the
// table, the set and the element first makes the deletion succeed whether or not
// they existed: after a host reboot, or with a table an earlier manager left,
// there is nothing to remove, and the next reconciliation replaces the table.
func RemoveSlotTransaction(slot Slot) Transaction {
	return Transaction{Nftables: []command{
		{Add: &object{Table: tableObjectOf()}},
		{Add: &object{Set: setObjectOf()}},
		{Add: &object{Element: elementObjectOf(slot)}},
		{Delete: &object{Element: elementObjectOf(slot)}},
	}}
}
