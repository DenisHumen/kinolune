// SPDX-License-Identifier: MPL-2.0

package netpolicy

import (
	"net"
	"net/netip"
	"testing"
	"time"
)

func TestClassify(t *testing.T) {
	cases := map[string]Zone{
		"127.0.0.1":          Loopback,
		"::1":                Loopback,
		"10.1.2.3":           LAN,
		"172.16.0.1":         LAN,
		"172.31.255.254":     LAN,
		"192.168.1.10":       LAN,
		"100.64.0.1":         LAN,
		"100.127.255.254":    LAN,
		"169.254.10.1":       LAN,
		"fe80::1":            LAN,
		"fd12:3456::1":       LAN,
		"fec0::1":            LAN,
		"::ffff:192.168.0.2": LAN,
		"::ffff:8.8.8.8":     Public,
		"8.8.8.8":            Public,
		"100.128.0.1":        Public,
		"172.32.0.1":         Public,
		"2001:db8::1":        Public,
		"0.0.0.0":            Invalid,
		"::":                 Invalid,
		"224.0.0.251":        Invalid,
		"ff02::fb":           Invalid,
		"255.255.255.255":    Invalid,
		"240.0.0.1":          Invalid,
		"0.1.2.3":            Invalid,
	}
	for s, want := range cases {
		if got := Classify(netip.MustParseAddr(s)); got != want {
			t.Errorf("Classify(%s) = %v, want %v", s, got, want)
		}
	}
}

func TestAllowed(t *testing.T) {
	lan := &net.TCPAddr{IP: net.ParseIP("192.168.1.5"), Port: 5000}
	pub := &net.TCPAddr{IP: net.ParseIP("203.0.113.9"), Port: 5000}
	if !Allowed(lan, false) || Allowed(pub, false) || !Allowed(pub, true) {
		t.Fatal("unexpected Allowed result")
	}
	if !Allowed(&net.UnixAddr{Name: "@x", Net: "unix"}, false) {
		t.Fatal("unix peers are local")
	}
	if Allowed(&net.TCPAddr{IP: net.IPv4zero}, true) {
		t.Fatal("unspecified peer must be refused")
	}
}

// fakeListener hands out pre-made connections with chosen remote addresses.
type fakeListener struct{ conns chan net.Conn }

func (f *fakeListener) Accept() (net.Conn, error) {
	c, ok := <-f.conns
	if !ok {
		return nil, net.ErrClosed
	}
	return c, nil
}
func (f *fakeListener) Close() error   { return nil }
func (f *fakeListener) Addr() net.Addr { return &net.TCPAddr{} }

type addrConn struct {
	net.Conn
	remote net.Addr
	closed bool
}

func (c *addrConn) RemoteAddr() net.Addr { return c.remote }
func (c *addrConn) Close() error         { c.closed = true; return nil }

func TestListenerDropsPublicPeers(t *testing.T) {
	a, _ := net.Pipe()
	pub := &addrConn{Conn: a, remote: &net.TCPAddr{IP: net.ParseIP("8.8.4.4"), Port: 1}}
	lan := &addrConn{Conn: a, remote: &net.TCPAddr{IP: net.ParseIP("10.0.0.7"), Port: 2}}
	fl := &fakeListener{conns: make(chan net.Conn, 2)}
	fl.conns <- pub
	fl.conns <- lan
	var rejected []net.Addr
	l := &Listener{Listener: fl, Rejected: func(a net.Addr) { rejected = append(rejected, a) }}
	done := make(chan net.Conn)
	go func() {
		c, err := l.Accept()
		if err != nil {
			t.Error(err)
		}
		done <- c
	}()
	select {
	case c := <-done:
		if c != lan {
			t.Fatalf("accepted %v, want the LAN peer", c.RemoteAddr())
		}
	case <-time.After(5 * time.Second):
		t.Fatal("Accept did not return")
	}
	if !pub.closed || len(rejected) != 1 {
		t.Fatalf("public peer not dropped: closed=%v rejected=%v", pub.closed, rejected)
	}
}
