// SPDX-License-Identifier: MPL-2.0

package system

import (
	"context"
	"fmt"

	"github.com/godbus/dbus/v5"
)

const (
	loginDest  = "org.freedesktop.login1"
	loginPath  = dbus.ObjectPath("/org/freedesktop/login1")
	loginIface = "org.freedesktop.login1.Manager"
)

// Inhibitor is a logind inhibitor lock ("something is going on").
type Inhibitor struct {
	What string `json:"what"`
	Who  string `json:"who"`
	Why  string `json:"why"`
	Mode string `json:"mode"`
	UID  uint32 `json:"uid"`
	PID  uint32 `json:"pid"`
}

// Logind reboots and powers off the machine.
type Logind struct{ Bus *Bus }

// Inhibitors lists the locks that would delay or block a shutdown.
func (l *Logind) Inhibitors(ctx context.Context) ([]Inhibitor, error) {
	var raw []struct {
		What, Who, Why, Mode string
		UID, PID             uint32
	}
	if err := l.Bus.call(ctx, loginDest, loginPath, loginIface+".ListInhibitors", []any{&raw}); err != nil {
		return nil, fmt.Errorf("logind inhibitors: %w", err)
	}
	out := []Inhibitor{}
	for _, r := range raw {
		out = append(out, Inhibitor{What: r.What, Who: r.Who, Why: r.Why, Mode: r.Mode, UID: r.UID, PID: r.PID})
	}
	return out, nil
}

// Reboot reboots now (non-interactive: polkit decides from the rule).
func (l *Logind) Reboot(ctx context.Context) error {
	if err := l.Bus.call(ctx, loginDest, loginPath, loginIface+".Reboot", nil, false); err != nil {
		return fmt.Errorf("reboot: %w", err)
	}
	return nil
}

// PowerOff powers the machine off now.
func (l *Logind) PowerOff(ctx context.Context) error {
	if err := l.Bus.call(ctx, loginDest, loginPath, loginIface+".PowerOff", nil, false); err != nil {
		return fmt.Errorf("power off: %w", err)
	}
	return nil
}
