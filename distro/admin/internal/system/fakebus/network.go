// SPDX-License-Identifier: MPL-2.0

package fakebus

import (
	"fmt"
	"slices"

	"github.com/godbus/dbus/v5"
)

// Fixture of the fake NetworkManager: wired connection active on enp1s0,
// a saved Wi-Fi profile, Wi-Fi radio on.
const (
	nmIface  = "org.freedesktop.NetworkManager"
	nmPath   = dbus.ObjectPath("/org/freedesktop/NetworkManager")
	devLo    = dbus.ObjectPath("/org/freedesktop/NetworkManager/Devices/1")
	devEth   = dbus.ObjectPath("/org/freedesktop/NetworkManager/Devices/2")
	devWifi  = dbus.ObjectPath("/org/freedesktop/NetworkManager/Devices/3")
	ip4Eth   = dbus.ObjectPath("/org/freedesktop/NetworkManager/IP4Config/2")
	connWire = dbus.ObjectPath("/org/freedesktop/NetworkManager/Settings/1")
	connWifi = dbus.ObjectPath("/org/freedesktop/NetworkManager/Settings/2")

	// WiredUUID and WifiUUID are the fixture connection UUIDs.
	WiredUUID = "5b1c3c1e-8f3e-4c39-9c8d-1f2a3b4c5d6e"
	WifiUUID  = "a7d2e0f4-2b6c-4e8a-9f1d-0c3b5a7e9d21"
)

type nmConn struct {
	path     dbus.ObjectPath
	uuid, id string
	typ      string
	device   dbus.ObjectPath
}

var nmConns = []nmConn{
	{connWire, WiredUUID, "Проводное подключение 1", "802-3-ethernet", devEth},
	{connWifi, WifiUUID, "HomeWiFi", "802-11-wireless", devWifi},
}

// maxActive bounds activations per test (their objects are exported up front).
const maxActive = 32

func activePath(n int) dbus.ObjectPath {
	return dbus.ObjectPath(fmt.Sprintf("/org/freedesktop/NetworkManager/ActiveConnection/%d", n))
}

func (f *Fake) networkManager() error {
	steps := []struct {
		path  dbus.ObjectPath
		iface string
		props map[string]any
	}{
		{nmPath, nmIface, map[string]any{
			"Version": "1.54.0", "State": uint32(70), "Connectivity": uint32(4),
			"NetworkingEnabled": true, "WirelessEnabled": true, "WirelessHardwareEnabled": true,
			"Devices":           []dbus.ObjectPath{devLo, devEth, devWifi},
			"ActiveConnections": []dbus.ObjectPath{activePath(1)},
		}},
		{devLo, nmIface + ".Device", map[string]any{
			"Interface": "lo", "DeviceType": uint32(32), "State": uint32(100), "HwAddress": "00:00:00:00:00:00",
			"ActiveConnection": dbus.ObjectPath("/"), "Ip4Config": dbus.ObjectPath("/"), "Ip6Config": dbus.ObjectPath("/"),
		}},
		{devEth, nmIface + ".Device", map[string]any{
			"Interface": "enp1s0", "DeviceType": uint32(1), "State": uint32(100), "HwAddress": "52:54:00:12:34:56",
			"ActiveConnection": activePath(1), "Ip4Config": ip4Eth, "Ip6Config": dbus.ObjectPath("/"),
		}},
		{devWifi, nmIface + ".Device", map[string]any{
			"Interface": "wlp2s0", "DeviceType": uint32(2), "State": uint32(30), "HwAddress": "52:54:00:ab:cd:ef",
			"ActiveConnection": dbus.ObjectPath("/"), "Ip4Config": dbus.ObjectPath("/"), "Ip6Config": dbus.ObjectPath("/"),
		}},
		{ip4Eth, nmIface + ".IP4Config", map[string]any{
			"AddressData":    []map[string]dbus.Variant{{"address": dbus.MakeVariant("192.168.1.20"), "prefix": dbus.MakeVariant(uint32(24))}},
			"Gateway":        "192.168.1.1",
			"NameserverData": []map[string]dbus.Variant{{"address": dbus.MakeVariant("192.168.1.1")}},
		}},
	}
	for _, s := range steps {
		if err := f.object(s.path, s.iface, s.props); err != nil {
			return err
		}
	}
	for n := 1; n <= maxActive; n++ {
		if err := f.exportProps(activePath(n)); err != nil {
			return err
		}
	}
	if err := f.activate(1, nmConns[0]); err != nil {
		return err
	}
	settingsOf := func(c nmConn) map[string]map[string]dbus.Variant {
		s := map[string]map[string]dbus.Variant{
			"connection": {"id": dbus.MakeVariant(c.id), "uuid": dbus.MakeVariant(c.uuid), "type": dbus.MakeVariant(c.typ)},
			"ipv4":       {"method": dbus.MakeVariant("auto")},
		}
		if c.typ == "802-11-wireless" {
			s["connection"]["autoconnect"] = dbus.MakeVariant(false)
			s["802-11-wireless"] = map[string]dbus.Variant{"ssid": dbus.MakeVariant([]byte("HomeWiFi"))}
		}
		return s
	}
	for _, c := range nmConns {
		c := c
		if err := f.conn.ExportMethodTable(map[string]any{
			"GetSettings": func() (map[string]map[string]dbus.Variant, *dbus.Error) { return settingsOf(c), nil },
		}, c.path, nmIface+".Settings.Connection"); err != nil {
			return err
		}
	}
	if err := f.conn.ExportMethodTable(map[string]any{
		"ListConnections": func() ([]dbus.ObjectPath, *dbus.Error) {
			return []dbus.ObjectPath{connWire, connWifi}, nil
		},
		"GetConnectionByUuid": func(uuid string) (dbus.ObjectPath, *dbus.Error) {
			for _, c := range nmConns {
				if c.uuid == uuid {
					return c.path, nil
				}
			}
			return "", dbus.NewError("org.freedesktop.NetworkManager.Settings.InvalidConnection", []any{"No connection with the UUID was found."})
		},
	}, "/org/freedesktop/NetworkManager/Settings", nmIface+".Settings"); err != nil {
		return err
	}
	next := 2
	return f.conn.ExportMethodTable(map[string]any{
		"ActivateConnection": func(conn, device, specific dbus.ObjectPath) (dbus.ObjectPath, *dbus.Error) {
			f.mu.Lock()
			full := next > maxActive
			f.mu.Unlock()
			if full {
				return "", dbus.MakeFailedError(fmt.Errorf("fake: too many activations"))
			}
			if err := f.record("ActivateConnection", conn); err != nil {
				return "", err
			}
			for _, c := range nmConns {
				if c.path == conn {
					f.mu.Lock()
					n := next
					next++
					f.mu.Unlock()
					if err := f.activate(n, c); err != nil {
						return "", dbus.MakeFailedError(err)
					}
					return activePath(n), nil
				}
			}
			return "", dbus.NewError("org.freedesktop.NetworkManager.UnknownConnection", []any{"unknown"})
		},
		"DeactivateConnection": func(active dbus.ObjectPath) *dbus.Error {
			if err := f.record("DeactivateConnection", active); err != nil {
				return err
			}
			f.mu.Lock()
			list, _ := f.props[nmPath][nmIface]["ActiveConnections"].Value().([]dbus.ObjectPath)
			i := slices.Index(list, active)
			if i < 0 {
				f.mu.Unlock()
				return dbus.NewError("org.freedesktop.NetworkManager.ConnectionNotActive", []any{"not active"})
			}
			devs, _ := f.props[active][nmIface+".Connection.Active"]["Devices"].Value().([]dbus.ObjectPath)
			f.setLocked(nmPath, nmIface, "ActiveConnections", slices.Delete(slices.Clone(list), i, i+1))
			for _, d := range devs {
				f.setLocked(d, nmIface+".Device", "ActiveConnection", dbus.ObjectPath("/"))
				f.setLocked(d, nmIface+".Device", "State", uint32(30))
			}
			f.mu.Unlock()
			f.dropObject(active)
			return nil
		},
	}, nmPath, nmIface)
}

// activate creates active connection n for c and links it to its device.
func (f *Fake) activate(n int, c nmConn) error {
	ap := activePath(n)
	if err := f.object(ap, nmIface+".Connection.Active", map[string]any{
		"Id": c.id, "Uuid": c.uuid, "Type": c.typ, "Devices": []dbus.ObjectPath{c.device}, "State": uint32(2),
		"Connection": c.path,
	}); err != nil {
		return err
	}
	f.mu.Lock()
	defer f.mu.Unlock()
	list, _ := f.props[nmPath][nmIface]["ActiveConnections"].Value().([]dbus.ObjectPath)
	if !slices.Contains(list, ap) {
		f.setLocked(nmPath, nmIface, "ActiveConnections", append(slices.Clone(list), ap))
	}
	f.setLocked(c.device, nmIface+".Device", "ActiveConnection", ap)
	f.setLocked(c.device, nmIface+".Device", "State", uint32(100))
	return nil
}
