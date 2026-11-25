// SPDX-License-Identifier: MPL-2.0

package fakebus

import (
	"slices"
	"sort"
	"strings"

	"github.com/godbus/dbus/v5"
)

const (
	fwIface = "org.fedoraproject.FirewallD1"
	fwPath  = dbus.ObjectPath("/org/fedoraproject/FirewallD1")
	// FwZonePath is the config object of the "home" zone.
	FwZonePath = dbus.ObjectPath("/org/fedoraproject/FirewallD1/config/zone/3")
)

// zoneSet is the content of one zone (services and "port/proto").
type zoneSet struct{ services, ports []string }

type fwState struct {
	runtime   map[string]*zoneSet
	permanent map[string]*zoneSet
}

// Zone returns the runtime and permanent services and ports of a zone.
func (f *Fake) Zone(name string) (rtServices, rtPorts, permServices, permPorts []string) {
	f.mu.Lock()
	defer f.mu.Unlock()
	rt, perm := f.fw.runtime[name], f.fw.permanent[name]
	return slices.Clone(rt.services), slices.Clone(rt.ports), slices.Clone(perm.services), slices.Clone(perm.ports)
}

func fwErr(code, msg string) *dbus.Error {
	return dbus.NewError("org.fedoraproject.FirewallD1.Exception", []any{code + ": " + msg})
}

func (f *Fake) firewalld() error {
	// The image's home zone (distro/Containerfile): samba, mdns, ws-discovery,
	// http, https, 8487/tcp; the stock ssh, dhcpv6-client stay.
	home := func() *zoneSet {
		return &zoneSet{
			services: []string{"dhcpv6-client", "http", "https", "mdns", "samba", "ssh", "ws-discovery"},
			ports:    []string{"8487/tcp"},
		}
	}
	f.fw = fwState{
		runtime:   map[string]*zoneSet{"home": home(), "public": {services: []string{"ssh"}}},
		permanent: map[string]*zoneSet{"home": home(), "public": {services: []string{"ssh"}}},
	}
	if err := f.object(fwPath, fwIface, map[string]any{"version": "2.3.1", "state": "RUNNING"}); err != nil {
		return err
	}
	if err := f.conn.ExportMethodTable(map[string]any{
		"getDefaultZone": func() (string, *dbus.Error) { return "home", nil },
	}, fwPath, fwIface); err != nil {
		return err
	}
	zone := func(name string) (*zoneSet, *dbus.Error) {
		z, ok := f.fw.runtime[name]
		if !ok {
			return nil, fwErr("INVALID_ZONE", name)
		}
		return z, nil
	}
	add := func(list *[]string, item, kind string) *dbus.Error {
		if slices.Contains(*list, item) {
			return fwErr("ALREADY_ENABLED", item)
		}
		*list = append(*list, item)
		sort.Strings(*list)
		return nil
	}
	remove := func(list *[]string, item string) *dbus.Error {
		i := slices.Index(*list, item)
		if i < 0 {
			return fwErr("NOT_ENABLED", item)
		}
		*list = slices.Delete(*list, i, i+1)
		return nil
	}
	if err := f.conn.ExportMethodTable(map[string]any{
		"getActiveZones": func() (map[string]map[string][]string, *dbus.Error) {
			return map[string]map[string][]string{"home": {"interfaces": {"enp1s0"}}}, nil
		},
		"getServices": func(z string) ([]string, *dbus.Error) {
			f.mu.Lock()
			defer f.mu.Unlock()
			zs, err := zone(z)
			if err != nil {
				return nil, err
			}
			return slices.Clone(zs.services), nil
		},
		"getPorts": func(z string) ([][]string, *dbus.Error) {
			f.mu.Lock()
			defer f.mu.Unlock()
			zs, err := zone(z)
			if err != nil {
				return nil, err
			}
			out := [][]string{}
			for _, p := range zs.ports {
				port, proto, _ := strings.Cut(p, "/")
				out = append(out, []string{port, proto})
			}
			return out, nil
		},
		"addService": func(z, svc string, timeout int32) (string, *dbus.Error) {
			if err := f.record("runtime.addService", z, svc); err != nil {
				return "", err
			}
			f.mu.Lock()
			defer f.mu.Unlock()
			zs, err := zone(z)
			if err != nil {
				return "", err
			}
			return z, add(&zs.services, svc, "service")
		},
		"removeService": func(z, svc string) (string, *dbus.Error) {
			if err := f.record("runtime.removeService", z, svc); err != nil {
				return "", err
			}
			f.mu.Lock()
			defer f.mu.Unlock()
			zs, err := zone(z)
			if err != nil {
				return "", err
			}
			return z, remove(&zs.services, svc)
		},
		"addPort": func(z, port, proto string, timeout int32) (string, *dbus.Error) {
			if err := f.record("runtime.addPort", z, port+"/"+proto); err != nil {
				return "", err
			}
			f.mu.Lock()
			defer f.mu.Unlock()
			zs, err := zone(z)
			if err != nil {
				return "", err
			}
			return z, add(&zs.ports, port+"/"+proto, "port")
		},
		"removePort": func(z, port, proto string) (string, *dbus.Error) {
			if err := f.record("runtime.removePort", z, port+"/"+proto); err != nil {
				return "", err
			}
			f.mu.Lock()
			defer f.mu.Unlock()
			zs, err := zone(z)
			if err != nil {
				return "", err
			}
			return z, remove(&zs.ports, port+"/"+proto)
		},
	}, fwPath, fwIface+".zone"); err != nil {
		return err
	}
	if err := f.conn.ExportMethodTable(map[string]any{
		"getZoneByName": func(z string) (dbus.ObjectPath, *dbus.Error) {
			if z != "home" {
				return "", fwErr("INVALID_ZONE", z)
			}
			return FwZonePath, nil
		},
	}, "/org/fedoraproject/FirewallD1/config", fwIface+".config"); err != nil {
		return err
	}
	perm := func() *zoneSet { return f.fw.permanent["home"] }
	return f.conn.ExportMethodTable(map[string]any{
		"addService": func(svc string) *dbus.Error {
			if err := f.record("permanent.addService", svc); err != nil {
				return err
			}
			f.mu.Lock()
			defer f.mu.Unlock()
			return add(&perm().services, svc, "service")
		},
		"removeService": func(svc string) *dbus.Error {
			if err := f.record("permanent.removeService", svc); err != nil {
				return err
			}
			f.mu.Lock()
			defer f.mu.Unlock()
			return remove(&perm().services, svc)
		},
		"addPort": func(port, proto string) *dbus.Error {
			if err := f.record("permanent.addPort", port+"/"+proto); err != nil {
				return err
			}
			f.mu.Lock()
			defer f.mu.Unlock()
			return add(&perm().ports, port+"/"+proto, "port")
		},
		"removePort": func(port, proto string) *dbus.Error {
			if err := f.record("permanent.removePort", port+"/"+proto); err != nil {
				return err
			}
			f.mu.Lock()
			defer f.mu.Unlock()
			return remove(&perm().ports, port+"/"+proto)
		},
	}, FwZonePath, fwIface+".config.zone")
}
