// SPDX-License-Identifier: MPL-2.0

package system

import (
	"context"
	"fmt"
	"slices"
	"sort"

	"github.com/godbus/dbus/v5"
)

const (
	fwDest       = "org.fedoraproject.FirewallD1"
	fwPath       = dbus.ObjectPath("/org/fedoraproject/FirewallD1")
	fwIface      = "org.fedoraproject.FirewallD1"
	fwConfigPath = dbus.ObjectPath("/org/fedoraproject/FirewallD1/config")
)

// Opening is a named set of firewalld services and ports of ours that the
// admin UI can open or close. The admin UI's own http/https cannot be closed
// from the admin UI (that would lock the user out).
type Opening struct {
	Name     string   `json:"name"`
	Title    string   `json:"title"`
	Services []string `json:"services,omitempty"`
	Ports    []string `json:"ports,omitempty"` // "8487/tcp"
	// Legacy services that meant the same in older images: they count as
	// open and are removed on close (smb was the "samba" service with the
	// NetBIOS ports 137–139 before DIST-018).
	Legacy []string `json:"-"`
}

// OurOpenings: media server, file sharing (SMB on 445 only, NetBIOS is
// off), discovery, Cockpit, SSH (05 §5.1–5.2, DIST-018).
var OurOpenings = []Opening{
	{Name: "media", Title: "Медиасервер", Ports: []string{"8487/tcp"}},
	{Name: "smb", Title: "Общие папки (SMB)", Ports: []string{"445/tcp"}, Legacy: []string{"samba"}},
	{Name: "discovery", Title: "Обнаружение в сети", Services: []string{"mdns", "ws-discovery"}},
	{Name: "cockpit", Title: "Расширенный режим (Cockpit)", Services: []string{"cockpit"}},
	{Name: "ssh", Title: "SSH", Services: []string{"ssh"}},
}

// FirewallStatus is what the admin UI shows.
type FirewallStatus struct {
	Running     bool              `json:"running"`
	Version     string            `json:"version,omitempty"`
	DefaultZone string            `json:"default_zone"`
	Zones       []FirewallZone    `json:"zones"`
	Openings    []OpeningStatus   `json:"openings"`
	Interfaces  map[string]string `json:"interfaces,omitempty"` // interface → zone
}

// FirewallZone is an active zone.
type FirewallZone struct {
	Name       string   `json:"name"`
	Interfaces []string `json:"interfaces,omitempty"`
	Sources    []string `json:"sources,omitempty"`
	Services   []string `json:"services"`
	Ports      []string `json:"ports"`
}

// OpeningStatus says whether one of ours is open in the default zone.
type OpeningStatus struct {
	Opening
	Open bool `json:"open"`
}

// Firewalld reads zones and opens/closes our ports, at runtime and
// permanently.
type Firewalld struct{ Bus *Bus }

// Status returns the active zones and our openings in the default zone.
func (f *Firewalld) Status(ctx context.Context) (FirewallStatus, error) {
	st := FirewallStatus{Zones: []FirewallZone{}, Openings: []OpeningStatus{}}
	if p, err := f.Bus.getAll(ctx, fwDest, fwPath, fwIface); err == nil {
		st.Version = str(p, "version")
		st.Running = str(p, "state") == "RUNNING"
	} else {
		return st, fmt.Errorf("firewalld: %w", err)
	}
	if err := f.Bus.call(ctx, fwDest, fwPath, fwIface+".getDefaultZone", []any{&st.DefaultZone}); err != nil {
		return st, fmt.Errorf("firewalld default zone: %w", err)
	}
	var active map[string]map[string][]string
	if err := f.Bus.call(ctx, fwDest, fwPath, fwIface+".zone.getActiveZones", []any{&active}); err != nil {
		return st, fmt.Errorf("firewalld active zones: %w", err)
	}
	names := make([]string, 0, len(active)+1)
	for z := range active {
		names = append(names, z)
	}
	if _, ok := active[st.DefaultZone]; !ok {
		names = append(names, st.DefaultZone)
	}
	sort.Strings(names)
	for _, z := range names {
		zone, err := f.zone(ctx, z)
		if err != nil {
			return st, err
		}
		zone.Interfaces = active[z]["interfaces"]
		zone.Sources = active[z]["sources"]
		st.Zones = append(st.Zones, zone)
		if z == st.DefaultZone {
			for _, o := range OurOpenings {
				st.Openings = append(st.Openings, OpeningStatus{Opening: o, Open: zoneHas(zone, o)})
			}
		}
	}
	return st, nil
}

func (f *Firewalld) zone(ctx context.Context, z string) (FirewallZone, error) {
	zone := FirewallZone{Name: z, Services: []string{}, Ports: []string{}}
	if err := f.Bus.call(ctx, fwDest, fwPath, fwIface+".zone.getServices", []any{&zone.Services}, z); err != nil {
		return zone, fmt.Errorf("zone %s services: %w", z, err)
	}
	var ports [][]string
	if err := f.Bus.call(ctx, fwDest, fwPath, fwIface+".zone.getPorts", []any{&ports}, z); err != nil {
		return zone, fmt.Errorf("zone %s ports: %w", z, err)
	}
	for _, p := range ports {
		if len(p) == 2 {
			zone.Ports = append(zone.Ports, p[0]+"/"+p[1])
		}
	}
	sort.Strings(zone.Services)
	sort.Strings(zone.Ports)
	return zone, nil
}

func zoneHas(z FirewallZone, o Opening) bool {
	for _, s := range o.Legacy {
		if slices.Contains(z.Services, s) {
			return true
		}
	}
	for _, s := range o.Services {
		if !slices.Contains(z.Services, s) {
			return false
		}
	}
	for _, p := range o.Ports {
		if !slices.Contains(z.Ports, p) {
			return false
		}
	}
	return true
}

// SetOpening opens or closes one of our openings in the default zone, both
// now (runtime) and after a reboot (permanent configuration).
func (f *Firewalld) SetOpening(ctx context.Context, name string, open bool) error {
	i := slices.IndexFunc(OurOpenings, func(o Opening) bool { return o.Name == name })
	if i < 0 {
		return errf(ErrNotFound, "opening %q", name)
	}
	o := OurOpenings[i]
	var zone string
	if err := f.Bus.call(ctx, fwDest, fwPath, fwIface+".getDefaultZone", []any{&zone}); err != nil {
		return fmt.Errorf("firewalld default zone: %w", err)
	}
	var cfgZone dbus.ObjectPath
	if err := f.Bus.call(ctx, fwDest, fwConfigPath, fwIface+".config.getZoneByName", []any{&cfgZone}, zone); err != nil {
		return fmt.Errorf("firewalld zone %s config: %w", zone, err)
	}
	for _, svc := range o.Services {
		if err := f.toggle(ctx, zone, cfgZone, "Service", open, svc); err != nil {
			return err
		}
	}
	for _, p := range o.Ports {
		port, proto, _ := cut(p)
		if err := f.toggle(ctx, zone, cfgZone, "Port", open, port, proto); err != nil {
			return err
		}
	}
	if !open {
		for _, svc := range o.Legacy {
			if err := f.toggle(ctx, zone, cfgZone, "Service", false, svc); err != nil {
				return err
			}
		}
	}
	return nil
}

// toggle adds or removes a service or port; "already enabled" and "not
// enabled" answers are not errors (the call is idempotent).
func (f *Firewalld) toggle(ctx context.Context, zone string, cfgZone dbus.ObjectPath, kind string, open bool, args ...any) error {
	verb := "remove"
	if open {
		verb = "add"
	}
	// Runtime: zone.addPort(zone, port, proto, timeout) / removePort(zone, port, proto).
	rt := append([]any{zone}, args...)
	if open {
		rt = append(rt, int32(0))
	}
	var ignored string
	if err := f.Bus.call(ctx, fwDest, fwPath, fwIface+".zone."+verb+kind, []any{&ignored}, rt...); err != nil && !alreadyDone(err) {
		return fmt.Errorf("firewalld %s%s %v: %w", verb, kind, args, err)
	}
	// Permanent: config.zone.addPort(port, proto) / removePort(port, proto).
	if err := f.Bus.call(ctx, fwDest, cfgZone, fwIface+".config.zone."+verb+kind, nil, args...); err != nil && !alreadyDone(err) {
		return fmt.Errorf("firewalld permanent %s%s %v: %w", verb, kind, args, err)
	}
	return nil
}

// alreadyDone recognises firewalld's ALREADY_ENABLED / NOT_ENABLED errors.
func alreadyDone(err error) bool {
	if de, ok := err.(*DBusError); ok {
		for _, s := range []string{"ALREADY_ENABLED", "NOT_ENABLED"} {
			if len(de.Message) >= len(s) && de.Message[:len(s)] == s {
				return true
			}
		}
	}
	return false
}

func cut(p string) (string, string, bool) {
	for i := len(p) - 1; i >= 0; i-- {
		if p[i] == '/' {
			return p[:i], p[i+1:], true
		}
	}
	return p, "tcp", false
}
