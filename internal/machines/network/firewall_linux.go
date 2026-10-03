//go:build linux

package network

import (
	"context"
	"encoding/binary"
	"net"
	"net/netip"

	"github.com/google/nftables"
	"github.com/google/nftables/expr"
	"github.com/vishvananda/netns"
	"golang.org/x/sys/unix"
)

const slotSetID uint32 = 1

// firewallBatch owns one transaction for the Cloud table. Explicit batch-local
// set IDs avoid the library's process-global automatic ID allocator.
type firewallBatch struct {
	conn   *nftables.Conn
	table  *nftables.Table
	nextID uint32
}

// applyFirewall applies one complete Cloud policy change in the caller's network namespace.
func applyFirewall(ctx context.Context, build func(*firewallBatch) error) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	namespace, err := netns.Get()
	if err != nil {
		return err
	}
	// This descriptor is read-only and only pins the namespace until Flush returns.
	defer func() { _ = namespace.Close() }()
	conn, err := nftables.New(nftables.WithNetNSFd(int(namespace)))
	if err != nil {
		return &FirewallError{Source: err}
	}
	batch := &firewallBatch{
		conn:   conn,
		table:  &nftables.Table{Family: nftables.TableFamilyINet, Name: "demi_cloud"},
		nextID: slotSetID,
	}
	if err := build(batch); err != nil {
		return &FirewallError{Source: err}
	}
	// Flush owns and closes its transient socket. Once sent, the batch drains
	// even if the caller cancels, so no kernel transaction outlives this call.
	if err := conn.Flush(); err != nil {
		return &FirewallError{Source: err}
	}
	return nil
}

// slots describes the Cloud interface/source-address admission set.
func (b *firewallBatch) slots() (*nftables.Set, error) {
	key, err := nftables.ConcatSetType(nftables.TypeIFName, nftables.TypeIPAddr)
	if err != nil {
		return nil, err
	}
	return &nftables.Set{Table: b.table, ID: slotSetID, Name: "slots", KeyType: key, Concatenation: true}, nil
}

// slotElement encodes one Cloud interface and IPv4 address in nft register layout.
func slotElement(slot Slot) nftables.SetElement {
	key := make([]byte, 20)
	copy(key, slot.HostInterface())
	copy(key[16:], slot.Address.AsSlice())
	return nftables.SetElement{Key: key}
}

// attach admits the Cloud slot before its sandbox runs.
func (b *firewallBatch) attach(slot Slot) error {
	set, err := b.slots()
	if err != nil {
		return err
	}
	return b.conn.SetAddElements(set, []nftables.SetElement{slotElement(slot)})
}

// detach removes a Cloud slot even after reboot or an earlier removal.
func (b *firewallBatch) detach(slot Slot) error {
	b.conn.AddTable(b.table)
	set, err := b.slots()
	if err != nil {
		return err
	}
	elements := []nftables.SetElement{slotElement(slot)}
	if err := b.conn.AddSet(set, elements); err != nil {
		return err
	}
	return b.conn.SetDeleteElements(set, elements)
}

// prepare replaces the entire Cloud policy and empties its admission set.
func (b *firewallBatch) prepare(pool netip.Prefix, backend []netip.Addr, port uint16, dns []netip.Addr) error {
	b.conn.AddTable(b.table)
	b.conn.DelTable(b.table)
	b.conn.AddTable(b.table)
	slots, err := b.slots()
	if err != nil {
		return err
	}
	if err := b.conn.AddSet(slots, nil); err != nil {
		return err
	}
	input, forward, ingress, nat := b.policyChains()
	for _, chain := range []*nftables.Chain{input, forward} {
		b.rule(
			chain,
			append(
				cloudInterface(expr.MetaKeyIIFNAME, expr.CmpOpEq),
				&expr.Verdict{Kind: expr.VerdictJump, Chain: "ingress"},
			)...)
	}
	established := cloudInterface(expr.MetaKeyOIFNAME, expr.CmpOpEq)
	established = append(
		established,
		&expr.Ct{Key: expr.CtKeySTATE, Register: 1},
		&expr.Bitwise{
			SourceRegister: 1,
			DestRegister:   1,
			Len:            4,
			Mask:           binary.NativeEndian.AppendUint32(nil, expr.CtStateBitESTABLISHED|expr.CtStateBitRELATED),
			Xor:            make([]byte, 4),
		},
		&expr.Cmp{Op: expr.CmpOpNeq, Register: 1, Data: make([]byte, 4)},
		&expr.Verdict{Kind: expr.VerdictAccept},
	)
	b.rule(forward, established...)
	b.rule(forward, append(cloudInterface(expr.MetaKeyOIFNAME, expr.CmpOpEq), &expr.Verdict{Kind: expr.VerdictDrop})...)
	if err := b.prepareIngress(ingress, slots, backend, port, dns); err != nil {
		return err
	}
	masquerade := []expr.Any{
		&expr.Payload{DestRegister: 1, Base: expr.PayloadBaseNetworkHeader, Offset: 12, Len: 4},
		&expr.Bitwise{
			SourceRegister: 1,
			DestRegister:   1,
			Len:            4,
			Mask:           net.CIDRMask(pool.Bits(), 32),
			Xor:            make([]byte, 4),
		},
		&expr.Cmp{Op: expr.CmpOpEq, Register: 1, Data: pool.Masked().Addr().AsSlice()},
	}
	masquerade = append(masquerade, cloudInterface(expr.MetaKeyOIFNAME, expr.CmpOpNeq)...)
	masquerade = append(masquerade, &expr.Masq{})
	b.ipv4Rule(nat, masquerade...)
	return nil
}

// rule queues one Cloud firewall rule in declaration order.
func (b *firewallBatch) rule(chain *nftables.Chain, expressions ...expr.Any) {
	b.conn.AddRule(&nftables.Rule{Table: b.table, Chain: chain, Exprs: expressions})
}

// cloudInterface matches the common prefix of every Cloud veth host end.
func cloudInterface(key expr.MetaKey, op expr.CmpOp) []expr.Any {
	return []expr.Any{
		&expr.Meta{Key: key, Register: 1},
		&expr.Cmp{Op: op, Register: 1, Data: []byte(hostInterfacePrefix)},
	}
}

// destinations matches the exact allowed Cloud backend or DNS IPv4 endpoints.
func (b *firewallBatch) destinations(addresses []netip.Addr) ([]expr.Any, error) {
	expressions := []expr.Any{&expr.Payload{DestRegister: 1, Base: expr.PayloadBaseNetworkHeader, Offset: 16, Len: 4}}
	if len(addresses) == 1 {
		return append(expressions, &expr.Cmp{Op: expr.CmpOpEq, Register: 1, Data: addresses[0].AsSlice()}), nil
	}
	elements := make([]nftables.SetElement, 0, len(addresses))
	for _, address := range addresses {
		elements = append(elements, nftables.SetElement{Key: address.AsSlice()})
	}
	lookup, err := b.constantSet(nftables.TypeIPAddr, elements, false)
	if err != nil {
		return nil, err
	}
	return append(expressions, lookup), nil
}

// constantSet queues an anonymous Cloud policy set and its lookup expression.
func (b *firewallBatch) constantSet(
	kind nftables.SetDatatype,
	elements []nftables.SetElement,
	interval bool,
) (*expr.Lookup, error) {
	b.nextID++
	set := &nftables.Set{
		Table:     b.table,
		ID:        b.nextID,
		Name:      "__set%d",
		Anonymous: true,
		Constant:  true,
		Interval:  interval,
		KeyType:   kind,
	}
	if err := b.conn.AddSet(set, elements); err != nil {
		return nil, err
	}
	return &expr.Lookup{SourceRegister: 1, SetID: set.ID, SetName: set.Name}, nil
}

// deniedDestinations encodes the Rust policy's denied ranges as half-open intervals.
func deniedDestinations() []nftables.SetElement {
	// The adjacent multicast and reserved /4 ranges are one /3, as nft's
	// interval normalization prints them. The zero end marks 2^32.
	prefixes := []string{
		"0.0.0.0/8",
		"10.0.0.0/8",
		"100.64.0.0/10",
		"127.0.0.0/8",
		"169.254.0.0/16",
		"172.16.0.0/12",
		"192.0.0.0/24",
		"192.0.2.0/24",
		"192.168.0.0/16",
		"198.18.0.0/15",
		"198.51.100.0/24",
		"203.0.113.0/24",
		"224.0.0.0/3",
	}
	elements := make([]nftables.SetElement, 0, len(prefixes)*2)
	for _, text := range prefixes {
		prefix := netip.MustParsePrefix(text)
		start := prefix.Addr().AsSlice()
		end := binary.BigEndian.Uint32(start) + uint32(uint64(1)<<(32-prefix.Bits()))
		elements = append(
			elements,
			nftables.SetElement{Key: start},
			nftables.SetElement{Key: binary.BigEndian.AppendUint32(nil, end), IntervalEnd: true},
		)
	}
	return elements
}

// ipv4Rule declares the protocol dependency of Cloud IPv4 payload expressions.
// nft needs this per rule, even after ingress's initial non-IPv4 rejection.
func (b *firewallBatch) ipv4Rule(chain *nftables.Chain, expressions ...expr.Any) {
	prefix := []expr.Any{
		&expr.Meta{Key: expr.MetaKeyNFPROTO, Register: 1},
		&expr.Cmp{Op: expr.CmpOpEq, Register: 1, Data: []byte{unix.NFPROTO_IPV4}},
	}
	b.rule(chain, append(prefix, expressions...)...)
}

func (b *firewallBatch) prepareIngress(
	ingress *nftables.Chain,
	slots *nftables.Set,
	backend []netip.Addr,
	port uint16,
	dns []netip.Addr,
) error {
	b.rule(
		ingress,
		&expr.Meta{Key: expr.MetaKeyNFPROTO, Register: 1},
		&expr.Cmp{Op: expr.CmpOpNeq, Register: 1, Data: []byte{unix.NFPROTO_IPV4}},
		&expr.Verdict{Kind: expr.VerdictDrop},
	)
	b.ipv4Rule(ingress,
		&expr.Meta{Key: expr.MetaKeyIIFNAME, Register: 1},
		&expr.Payload{DestRegister: 2, Base: expr.PayloadBaseNetworkHeader, Offset: 12, Len: 4},
		&expr.Lookup{SourceRegister: 1, SetID: slots.ID, SetName: slots.Name, Invert: true},
		&expr.Verdict{Kind: expr.VerdictDrop})
	if err := b.allowBackend(ingress, backend, port); err != nil {
		return err
	}
	dnsMatch, err := b.destinations(dns)
	if err != nil {
		return err
	}
	protocols, err := b.constantSet(
		nftables.TypeInetProto,
		[]nftables.SetElement{{Key: []byte{unix.IPPROTO_TCP}}, {Key: []byte{unix.IPPROTO_UDP}}},
		false,
	)
	if err != nil {
		return err
	}
	dnsMatch = append(dnsMatch,
		&expr.Meta{Key: expr.MetaKeyL4PROTO, Register: 1}, protocols,
		&expr.Payload{DestRegister: 1, Base: expr.PayloadBaseTransportHeader, Offset: 2, Len: 2},
		&expr.Cmp{Op: expr.CmpOpEq, Register: 1, Data: binary.BigEndian.AppendUint16(nil, 53)},
		&expr.Verdict{Kind: expr.VerdictAccept})
	b.ipv4Rule(ingress, dnsMatch...)
	b.rule(
		ingress,
		&expr.Fib{Register: 1, ResultADDRTYPE: true, FlagDADDR: true},
		&expr.Cmp{Op: expr.CmpOpEq, Register: 1, Data: binary.NativeEndian.AppendUint32(nil, unix.RTN_LOCAL)},
		&expr.Verdict{Kind: expr.VerdictDrop},
	)
	denied, err := b.constantSet(nftables.TypeIPAddr, deniedDestinations(), true)
	if err != nil {
		return err
	}
	b.ipv4Rule(
		ingress,
		&expr.Payload{DestRegister: 1, Base: expr.PayloadBaseNetworkHeader, Offset: 16, Len: 4},
		denied,
		&expr.Verdict{Kind: expr.VerdictDrop},
	)
	b.rule(ingress, &expr.Verdict{Kind: expr.VerdictAccept})
	return nil
}

func (b *firewallBatch) policyChains() (input, forward, ingress, nat *nftables.Chain) {
	accept := nftables.ChainPolicyAccept
	input = b.conn.AddChain(
		&nftables.Chain{
			Table:    b.table,
			Name:     "input",
			Type:     nftables.ChainTypeFilter,
			Hooknum:  nftables.ChainHookInput,
			Priority: nftables.ChainPriorityRef(-10),
			Policy:   &accept,
		},
	)
	forward = b.conn.AddChain(
		&nftables.Chain{
			Table:    b.table,
			Name:     "forward",
			Type:     nftables.ChainTypeFilter,
			Hooknum:  nftables.ChainHookForward,
			Priority: nftables.ChainPriorityRef(-10),
			Policy:   &accept,
		},
	)
	ingress = b.conn.AddChain(&nftables.Chain{Table: b.table, Name: "ingress"})
	nat = b.conn.AddChain(
		&nftables.Chain{
			Table:    b.table,
			Name:     "nat",
			Type:     nftables.ChainTypeNAT,
			Hooknum:  nftables.ChainHookPostrouting,
			Priority: nftables.ChainPriorityNATSource,
			Policy:   &accept,
		},
	)
	return input, forward, ingress, nat
}

func (b *firewallBatch) allowBackend(ingress *nftables.Chain, backend []netip.Addr, port uint16) error {
	backendMatch, err := b.destinations(backend)
	if err != nil {
		return err
	}
	backendMatch = append(backendMatch,
		&expr.Meta{Key: expr.MetaKeyL4PROTO, Register: 1},
		&expr.Cmp{Op: expr.CmpOpEq, Register: 1, Data: []byte{unix.IPPROTO_TCP}},
		&expr.Payload{DestRegister: 1, Base: expr.PayloadBaseTransportHeader, Offset: 2, Len: 2},
		&expr.Cmp{Op: expr.CmpOpEq, Register: 1, Data: binary.BigEndian.AppendUint16(nil, port)},
		&expr.Verdict{Kind: expr.VerdictAccept})
	b.ipv4Rule(ingress, backendMatch...)
	return nil
}
