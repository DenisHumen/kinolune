// SPDX-License-Identifier: MPL-2.0

// Package fakebus starts a private dbus-daemon and serves fake systemd,
// logind, NetworkManager, firewalld and UDisks2 on it, with the same bus
// names, object paths, interfaces and signatures as the real services, so
// the D-Bus clients in internal/system are tested over a real bus without
// touching the machine. Used by tests only.
package fakebus

import (
	"bufio"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/godbus/dbus/v5"
)

// Fake is a running private bus with the fake services.
type Fake struct {
	// Addr is the bus address for dbus.Connect.
	Addr string

	conn *dbus.Conn
	mu   sync.Mutex
	// props[path][iface][name]
	props map[dbus.ObjectPath]map[string]map[string]dbus.Variant
	calls []string
	deny  map[string]bool
	// exported paths with the Properties interface.
	exported map[dbus.ObjectPath]bool

	fw fwState
}

// Start runs dbus-daemon on a socket in a temporary directory. Without
// dbus-daemon the test is skipped, unless ALPHA_ADMIN_REQUIRE_DBUS=1 (CI).
func Start(t testing.TB) *Fake {
	t.Helper()
	bin, err := exec.LookPath("dbus-daemon")
	if err != nil {
		if os.Getenv("ALPHA_ADMIN_REQUIRE_DBUS") == "1" {
			t.Fatal("dbus-daemon not found")
		}
		t.Skip("dbus-daemon not found; install dbus to run the D-Bus tests")
	}
	// Short path: unix socket paths are limited to 108 bytes.
	dir, err := os.MkdirTemp("", "fb")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { os.RemoveAll(dir) })
	cfg := filepath.Join(dir, "bus.conf")
	if err := os.WriteFile(cfg, []byte(`<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:path=`+filepath.Join(dir, "bus")+`</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
`), 0o600); err != nil {
		t.Fatal(err)
	}
	cmd := exec.Command(bin, "--config-file="+cfg, "--nofork", "--print-address=1")
	out, err := cmd.StdoutPipe()
	if err != nil {
		t.Fatal(err)
	}
	cmd.Stderr = os.Stderr
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = cmd.Process.Kill(); _ = cmd.Wait() })
	addrCh := make(chan string, 1)
	go func() {
		line, _ := bufio.NewReader(out).ReadString('\n')
		addrCh <- strings.TrimSpace(line)
	}()
	var addr string
	select {
	case addr = <-addrCh:
	case <-time.After(10 * time.Second):
		t.Fatal("dbus-daemon did not start")
	}
	if addr == "" {
		t.Fatal("dbus-daemon printed no address")
	}
	conn, err := dbus.Connect(addr)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { conn.Close() })
	f := &Fake{Addr: addr, conn: conn, props: map[dbus.ObjectPath]map[string]map[string]dbus.Variant{}, deny: map[string]bool{}, exported: map[dbus.ObjectPath]bool{}}
	for _, name := range []string{"org.freedesktop.systemd1", "org.freedesktop.login1", "org.freedesktop.NetworkManager", "org.fedoraproject.FirewallD1", "org.freedesktop.UDisks2"} {
		reply, err := conn.RequestName(name, dbus.NameFlagDoNotQueue)
		if err != nil || reply != dbus.RequestNameReplyPrimaryOwner {
			t.Fatalf("own %s: %v %v", name, reply, err)
		}
	}
	for _, setup := range []func() error{f.systemd, f.logind, f.networkManager, f.firewalld, f.udisks} {
		if err := setup(); err != nil {
			t.Fatal(err)
		}
	}
	return f
}

// Client connects a new client to the bus.
func (f *Fake) Client(t testing.TB) *dbus.Conn {
	t.Helper()
	c, err := dbus.Connect(f.Addr)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { c.Close() })
	return c
}

// Calls returns the changing calls made so far ("RestartUnit smb.service").
func (f *Fake) Calls() []string {
	f.mu.Lock()
	defer f.mu.Unlock()
	return append([]string(nil), f.calls...)
}

// Deny makes a method answer AccessDenied, as polkit would.
func (f *Fake) Deny(method string) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.deny[method] = true
}

func (f *Fake) record(method string, args ...any) *dbus.Error {
	f.mu.Lock()
	defer f.mu.Unlock()
	if f.deny[method] {
		return dbus.NewError("org.freedesktop.DBus.Error.AccessDenied", []any{"Access denied (fake polkit)"})
	}
	parts := []string{method}
	for _, a := range args {
		parts = append(parts, fmt.Sprint(a))
	}
	f.calls = append(f.calls, strings.Join(parts, " "))
	return nil
}

// Prop returns a property value (for assertions).
func (f *Fake) Prop(path dbus.ObjectPath, iface, name string) any {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.props[path][iface][name].Value()
}

// SetProp changes a property (for test setup).
func (f *Fake) SetProp(path dbus.ObjectPath, iface, name string, v any) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.setLocked(path, iface, name, v)
}

func (f *Fake) setLocked(path dbus.ObjectPath, iface, name string, v any) {
	if f.props[path] == nil {
		f.props[path] = map[string]map[string]dbus.Variant{}
	}
	if f.props[path][iface] == nil {
		f.props[path][iface] = map[string]dbus.Variant{}
	}
	f.props[path][iface][name] = dbus.MakeVariant(v)
}

// object registers properties of iface on path (and the Properties
// interface on first use).
func (f *Fake) object(path dbus.ObjectPath, iface string, props map[string]any) error {
	f.mu.Lock()
	for k, v := range props {
		f.setLocked(path, iface, k, v)
	}
	if f.props[path] == nil {
		f.props[path] = map[string]map[string]dbus.Variant{}
	}
	if f.props[path][iface] == nil {
		f.props[path][iface] = map[string]dbus.Variant{}
	}
	f.mu.Unlock()
	return f.exportProps(path)
}

// exportProps exports the Properties interface on path once. Objects that
// appear while the bus runs (active connections) are exported up front:
// godbus' export table must not change while it dispatches calls.
func (f *Fake) exportProps(path dbus.ObjectPath) error {
	f.mu.Lock()
	done := f.exported[path]
	f.exported[path] = true
	f.mu.Unlock()
	if done {
		return nil
	}
	return f.conn.Export(&properties{f: f, path: path}, path, "org.freedesktop.DBus.Properties")
}

func (f *Fake) dropObject(path dbus.ObjectPath) {
	f.mu.Lock()
	delete(f.props, path)
	f.mu.Unlock()
}

type properties struct {
	f    *Fake
	path dbus.ObjectPath
}

func (p *properties) Get(iface, name string) (dbus.Variant, *dbus.Error) {
	p.f.mu.Lock()
	defer p.f.mu.Unlock()
	v, ok := p.f.props[p.path][iface][name]
	if !ok {
		return dbus.Variant{}, dbus.NewError("org.freedesktop.DBus.Error.UnknownProperty", []any{name})
	}
	return v, nil
}

func (p *properties) GetAll(iface string) (map[string]dbus.Variant, *dbus.Error) {
	p.f.mu.Lock()
	defer p.f.mu.Unlock()
	all, ok := p.f.props[p.path]
	if !ok {
		return nil, dbus.NewError("org.freedesktop.DBus.Error.UnknownObject", []any{string(p.path)})
	}
	out := map[string]dbus.Variant{}
	for k, v := range all[iface] {
		out[k] = v
	}
	return out, nil
}

func (p *properties) Set(iface, name string, v dbus.Variant) *dbus.Error {
	if err := p.f.record("Set", iface+"."+name, v.Value()); err != nil {
		return err
	}
	p.f.mu.Lock()
	defer p.f.mu.Unlock()
	if _, ok := p.f.props[p.path][iface][name]; !ok {
		return dbus.NewError("org.freedesktop.DBus.Error.UnknownProperty", []any{name})
	}
	p.f.props[p.path][iface][name] = v
	return nil
}
