// SPDX-License-Identifier: MPL-2.0

// Package system talks to the OS over the system D-Bus: systemd, logind,
// NetworkManager, firewalld and UDisks2 (05 §5.1). The admin daemon runs as
// an unprivileged system user; every changing call is authorised by polkit
// rules shipped in the image (usr/share/polkit-1/rules.d/50-alpha-admin.rules)
// that allow exactly these actions for that user and nothing else. Which
// human may trigger them is decided before, by the role check of the HTTP
// layer.
package system

import (
	"context"
	"errors"
	"fmt"
	"time"

	"github.com/godbus/dbus/v5"
)

// Errors that the HTTP layer maps to status codes.
var (
	// ErrNotFound: no such unit, connection, block device…
	ErrNotFound = errors.New("not found")
	// ErrForbidden: the object exists but is not ours to touch.
	ErrForbidden = errors.New("not allowed")
	// ErrConflict: the request does not fit the current state.
	ErrConflict = errors.New("conflict")
)

// Bus wraps a D-Bus connection (the system bus in production, a private bus
// with fake services in tests).
type Bus struct {
	Conn *dbus.Conn
	// Timeout of one call.
	Timeout time.Duration
}

// SystemBus connects to the system bus.
func SystemBus() (*Bus, error) {
	conn, err := dbus.ConnectSystemBus()
	if err != nil {
		return nil, err
	}
	return &Bus{Conn: conn, Timeout: 25 * time.Second}, nil
}

// Close closes the connection.
func (b *Bus) Close() error { return b.Conn.Close() }

func (b *Bus) ctx(ctx context.Context) (context.Context, context.CancelFunc) {
	t := b.Timeout
	if t <= 0 {
		t = 25 * time.Second
	}
	return context.WithTimeout(ctx, t)
}

// call invokes method on dest/path and stores the reply in out.
func (b *Bus) call(ctx context.Context, dest string, path dbus.ObjectPath, method string, out []any, args ...any) error {
	ctx, cancel := b.ctx(ctx)
	defer cancel()
	c := b.Conn.Object(dest, path).CallWithContext(ctx, method, 0, args...)
	if c.Err != nil {
		return wrapErr(c.Err)
	}
	if len(out) > 0 {
		return c.Store(out...)
	}
	return nil
}

// getAll reads all properties of iface.
func (b *Bus) getAll(ctx context.Context, dest string, path dbus.ObjectPath, iface string) (map[string]dbus.Variant, error) {
	var props map[string]dbus.Variant
	err := b.call(ctx, dest, path, "org.freedesktop.DBus.Properties.GetAll", []any{&props}, iface)
	return props, err
}

func (b *Bus) setProp(ctx context.Context, dest string, path dbus.ObjectPath, iface, name string, v any) error {
	return b.call(ctx, dest, path, "org.freedesktop.DBus.Properties.Set", nil, iface, name, dbus.MakeVariant(v))
}

// DBusError is a D-Bus error reply; Name is the error name
// (org.freedesktop.DBus.Error.AccessDenied…).
type DBusError struct {
	Name    string
	Message string
}

func (e *DBusError) Error() string {
	if e.Message == "" {
		return e.Name
	}
	return e.Name + ": " + e.Message
}

// Unwrap maps well-known names to the package errors.
func (e *DBusError) Unwrap() error {
	switch e.Name {
	case "org.freedesktop.DBus.Error.AccessDenied", "org.freedesktop.PolicyKit1.Error.NotAuthorized",
		"org.freedesktop.UDisks2.Error.NotAuthorized", "org.freedesktop.UDisks2.Error.NotAuthorizedCanObtain",
		"org.freedesktop.NetworkManager.PermissionDenied", "org.freedesktop.login1.NotAuthorized",
		"org.freedesktop.DBus.Error.InteractiveAuthorizationRequired":
		return ErrForbidden
	case "org.freedesktop.systemd1.NoSuchUnit", "org.freedesktop.NetworkManager.UnknownConnection",
		"org.freedesktop.NetworkManager.Settings.InvalidConnection",
		"org.freedesktop.DBus.Error.UnknownObject", "org.freedesktop.DBus.Error.UnknownMethod":
		return ErrNotFound
	case "org.freedesktop.NetworkManager.ConnectionNotActive", "org.freedesktop.UDisks2.Error.AlreadyMounted",
		"org.freedesktop.UDisks2.Error.NotMounted", "org.freedesktop.UDisks2.Error.DeviceBusy":
		return ErrConflict
	}
	return nil
}

func wrapErr(err error) error {
	var de dbus.Error
	if errors.As(err, &de) {
		msg := ""
		if len(de.Body) > 0 {
			msg, _ = de.Body[0].(string)
		}
		return &DBusError{Name: de.Name, Message: msg}
	}
	var dep *dbus.Error
	if errors.As(err, &dep) && dep != nil {
		msg := ""
		if len(dep.Body) > 0 {
			msg, _ = dep.Body[0].(string)
		}
		return &DBusError{Name: dep.Name, Message: msg}
	}
	return err
}

// Typed property readers: a missing or mistyped property reads as the zero
// value, so an older daemon without a property does not break a page.

func str(p map[string]dbus.Variant, k string) string {
	v, _ := p[k].Value().(string)
	return v
}

func boolean(p map[string]dbus.Variant, k string) bool {
	v, _ := p[k].Value().(bool)
	return v
}

func u32(p map[string]dbus.Variant, k string) uint32 {
	v, _ := p[k].Value().(uint32)
	return v
}

func u64(p map[string]dbus.Variant, k string) uint64 {
	switch v := p[k].Value().(type) {
	case uint64:
		return v
	case uint32:
		return uint64(v)
	case uint16:
		return uint64(v)
	}
	return 0
}

func i64(p map[string]dbus.Variant, k string) int64 {
	switch v := p[k].Value().(type) {
	case int64:
		return v
	case int32:
		return int64(v)
	}
	return 0
}

func f64(p map[string]dbus.Variant, k string) float64 {
	v, _ := p[k].Value().(float64)
	return v
}

func objPath(p map[string]dbus.Variant, k string) dbus.ObjectPath {
	v, _ := p[k].Value().(dbus.ObjectPath)
	return v
}

func objPaths(p map[string]dbus.Variant, k string) []dbus.ObjectPath {
	v, _ := p[k].Value().([]dbus.ObjectPath)
	return v
}

func strs(p map[string]dbus.Variant, k string) []string {
	v, _ := p[k].Value().([]string)
	return v
}

// byteString decodes a NUL-terminated "ay" (UDisks paths).
func byteString(b []byte) string {
	for i, c := range b {
		if c == 0 {
			return string(b[:i])
		}
	}
	return string(b)
}

func usecTime(us uint64) *time.Time {
	if us == 0 {
		return nil
	}
	t := time.UnixMicro(int64(us)).UTC()
	return &t
}

func errf(base error, format string, args ...any) error {
	return fmt.Errorf("%w: %s", base, fmt.Sprintf(format, args...))
}
