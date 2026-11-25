// SPDX-License-Identifier: MPL-2.0

// Package netpolicy decides which peers may talk to the admin backend.
//
// By default the admin UI is reachable from the local network only, with the
// same address classes the media server treats as "LAN" (pa-vfs-http Zone):
// loopback, RFC 1918, carrier-grade NAT 100.64.0.0/10 (Tailscale and similar
// overlays), IPv6 unique local fc00::/7 and site-local fec0::/10. On top of
// that, link-local addresses (169.254.0.0/16, fe80::/10) count as LAN for an
// incoming peer: a laptop plugged straight into the NAS has nothing else.
package netpolicy

import (
	"net"
	"net/netip"
)

// Zone is how public a peer address is.
type Zone int

const (
	// Invalid addresses: unspecified, multicast, broadcast, reserved.
	Invalid Zone = iota
	// Loopback is this device.
	Loopback
	// LAN is the local network (see the package docs).
	LAN
	// Public is everything else.
	Public
)

func (z Zone) String() string {
	switch z {
	case Loopback:
		return "loopback"
	case LAN:
		return "lan"
	case Public:
		return "public"
	default:
		return "invalid"
	}
}

var (
	cgnat     = netip.MustParsePrefix("100.64.0.0/10")
	ula       = netip.MustParsePrefix("fc00::/7")
	siteLocal = netip.MustParsePrefix("fec0::/10")
)

// Classify returns the zone of ip. IPv4-mapped IPv6 addresses are classified
// as the IPv4 address they carry.
func Classify(ip netip.Addr) Zone {
	ip = ip.Unmap()
	if !ip.IsValid() || ip.IsUnspecified() || ip.IsMulticast() {
		return Invalid
	}
	if ip.Is4() {
		b := ip.As4()
		if b == [4]byte{255, 255, 255, 255} || b[0] == 0 || b[0] >= 240 {
			return Invalid
		}
	}
	switch {
	case ip.IsLoopback():
		return Loopback
	case ip.IsPrivate(), ip.IsLinkLocalUnicast(), cgnat.Contains(ip), ula.Contains(ip), siteLocal.Contains(ip):
		return LAN
	}
	return Public
}

// Allowed reports whether a peer at addr may connect. With allowPublic only
// invalid addresses are refused.
func Allowed(addr net.Addr, allowPublic bool) bool {
	ap, ok := addrPort(addr)
	if !ok {
		// Unix sockets and the like: local by construction.
		_, unix := addr.(*net.UnixAddr)
		return unix
	}
	switch Classify(ap.Addr()) {
	case Loopback, LAN:
		return true
	case Public:
		return allowPublic
	}
	return false
}

func addrPort(addr net.Addr) (netip.AddrPort, bool) {
	switch a := addr.(type) {
	case *net.TCPAddr:
		return a.AddrPort(), true
	case *net.UDPAddr:
		return a.AddrPort(), true
	}
	if addr == nil {
		return netip.AddrPort{}, false
	}
	ap, err := netip.ParseAddrPort(addr.String())
	return ap, err == nil
}

// Listener drops connections from peers that are not allowed before any byte
// (and any TLS handshake) is exchanged. Rejected is called for each dropped
// peer (may be nil).
type Listener struct {
	net.Listener
	AllowPublic bool
	Rejected    func(net.Addr)
}

// Accept returns the next allowed connection.
func (l *Listener) Accept() (net.Conn, error) {
	for {
		c, err := l.Listener.Accept()
		if err != nil {
			return nil, err
		}
		if Allowed(c.RemoteAddr(), l.AllowPublic) {
			return c, nil
		}
		if l.Rejected != nil {
			l.Rejected(c.RemoteAddr())
		}
		_ = c.Close()
	}
}
