// SPDX-License-Identifier: MPL-2.0

package fakebus

import (
	"fmt"
	"strings"
	"sync/atomic"

	"github.com/godbus/dbus/v5"
)

// Fixture timestamps (µs since the epoch): 2026-11-25 08:00:00 UTC.
const since = uint64(1795593600) * 1_000_000

// Units of the fake systemd: name → ActiveState, SubState, UnitFileState.
var fakeUnits = map[string][3]string{
	"alpha-media.service":    {"active", "running", "enabled"},
	"alpha-admin.service":    {"active", "running", "enabled"},
	"alpha-updated.service":  {"active", "running", "enabled"},
	"smb.service":            {"failed", "failed", "enabled"},
	"wsdd.service":           {"active", "running", "enabled"},
	"avahi-daemon.service":   {"active", "running", "enabled"},
	"smartd.service":         {"active", "running", "enabled"},
	"chronyd.service":        {"active", "running", "enabled"},
	"NetworkManager.service": {"active", "running", "enabled"},
	"firewalld.service":      {"active", "running", "enabled"},
	"sshd.service":           {"inactive", "dead", "disabled"},
	"cockpit.socket":         {"inactive", "dead", "disabled"},
	"getty@tty1.service":     {"active", "running", "enabled"},
}

// UnitPath is the object path of a unit (systemd's escaping).
func UnitPath(name string) dbus.ObjectPath {
	var b strings.Builder
	for i := 0; i < len(name); i++ {
		c := name[i]
		if (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9' && i > 0) {
			b.WriteByte(c)
		} else {
			fmt.Fprintf(&b, "_%02x", c)
		}
	}
	return dbus.ObjectPath("/org/freedesktop/systemd1/unit/" + b.String())
}

func (f *Fake) addUnit(name, active, sub, file string) error {
	load := "loaded"
	if file == "" {
		load = "not-found"
	}
	p := UnitPath(name)
	if err := f.object(p, "org.freedesktop.systemd1.Unit", map[string]any{
		"Id": name, "Description": "Fake " + name, "LoadState": load,
		"ActiveState": active, "SubState": sub, "UnitFileState": file,
		"ActiveEnterTimestamp": since, "InactiveEnterTimestamp": since - 60_000_000,
	}); err != nil {
		return err
	}
	if strings.HasSuffix(name, ".service") && load == "loaded" {
		pid := uint32(0)
		if active == "active" {
			pid = 1234
		}
		result := "success"
		if active == "failed" {
			result = "exit-code"
		}
		return f.object(p, "org.freedesktop.systemd1.Service", map[string]any{
			"MainPID": pid, "NRestarts": uint32(2), "Result": result,
		})
	}
	return nil
}

func (f *Fake) systemd() error {
	for name, s := range fakeUnits {
		if err := f.addUnit(name, s[0], s[1], s[2]); err != nil {
			return err
		}
	}
	var jobs atomic.Int32
	return f.conn.ExportMethodTable(map[string]any{
		"LoadUnit": func(name string) (dbus.ObjectPath, *dbus.Error) {
			p := UnitPath(name)
			f.mu.Lock()
			_, ok := f.props[p]
			f.mu.Unlock()
			if !ok {
				// Real systemd hands out a "not-found" unit object here; the
				// fake does not create objects while serving (godbus' export
				// table is not safe to change during dispatch).
				return "", dbus.NewError("org.freedesktop.systemd1.NoSuchUnit", []any{"Unit " + name + " not found."})
			}
			return p, nil
		},
		"RestartUnit": func(name, mode string) (dbus.ObjectPath, *dbus.Error) {
			if err := f.record("RestartUnit", name, mode); err != nil {
				return "", err
			}
			f.SetProp(UnitPath(name), "org.freedesktop.systemd1.Unit", "ActiveState", "active")
			f.SetProp(UnitPath(name), "org.freedesktop.systemd1.Unit", "SubState", "running")
			return dbus.ObjectPath(fmt.Sprintf("/org/freedesktop/systemd1/job/%d", jobs.Add(1))), nil
		},
	}, "/org/freedesktop/systemd1", "org.freedesktop.systemd1.Manager")
}

// Inhibitor of the fake logind: the media server says a film is playing.
type inhibitor struct {
	What, Who, Why, Mode string
	UID, PID             uint32
}

func (f *Fake) logind() error {
	return f.conn.ExportMethodTable(map[string]any{
		"ListInhibitors": func() ([]inhibitor, *dbus.Error) {
			return []inhibitor{{"shutdown:sleep", "alpha-media", "Playing: Film on TV", "delay", 990, 4321}}, nil
		},
		"Reboot": func(interactive bool) *dbus.Error {
			return f.record("Reboot", interactive)
		},
		"PowerOff": func(interactive bool) *dbus.Error {
			return f.record("PowerOff", interactive)
		},
	}, "/org/freedesktop/login1", "org.freedesktop.login1.Manager")
}
