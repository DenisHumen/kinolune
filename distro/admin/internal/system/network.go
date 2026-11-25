// SPDX-License-Identifier: MPL-2.0

package system

import (
	"context"
	"fmt"

	"github.com/godbus/dbus/v5"
)

const (
	nmDest     = "org.freedesktop.NetworkManager"
	nmPath     = dbus.ObjectPath("/org/freedesktop/NetworkManager")
	nmIface    = "org.freedesktop.NetworkManager"
	nmSettings = dbus.ObjectPath("/org/freedesktop/NetworkManager/Settings")
)

// NetworkStatus is the overview of NetworkManager.
type NetworkStatus struct {
	Version           string          `json:"version"`
	State             string          `json:"state"`
	Connectivity      string          `json:"connectivity"`
	NetworkingEnabled bool            `json:"networking_enabled"`
	WirelessEnabled   bool            `json:"wireless_enabled"`
	WirelessHardware  bool            `json:"wireless_hardware_enabled"`
	Devices           []NetworkDevice `json:"devices"`
}

// NetworkDevice is one interface.
type NetworkDevice struct {
	Interface  string   `json:"interface"`
	Type       string   `json:"type"`
	State      string   `json:"state"`
	HwAddress  string   `json:"hw_address,omitempty"`
	Connection string   `json:"connection,omitempty"` // active connection id
	IPv4       []string `json:"ipv4,omitempty"`       // "192.168.1.10/24"
	Gateway4   string   `json:"gateway4,omitempty"`
	DNS4       []string `json:"dns4,omitempty"`
	IPv6       []string `json:"ipv6,omitempty"`
}

// Connection is a saved connection profile (never with secrets).
type Connection struct {
	UUID        string `json:"uuid"`
	ID          string `json:"id"`
	Type        string `json:"type"`
	Autoconnect bool   `json:"autoconnect"`
	IPv4Method  string `json:"ipv4_method,omitempty"`
	Active      bool   `json:"active"`
	Device      string `json:"device,omitempty"`
}

// NetworkManager reads network state and switches connections.
type NetworkManager struct{ Bus *Bus }

var nmStates = map[uint32]string{
	10: "asleep", 20: "disconnected", 30: "disconnecting", 40: "connecting",
	50: "connected-local", 60: "connected-site", 70: "connected-global",
}

var nmConnectivity = map[uint32]string{1: "none", 2: "portal", 3: "limited", 4: "full"}

var nmDeviceTypes = map[uint32]string{
	1: "ethernet", 2: "wifi", 5: "bluetooth", 10: "bond", 11: "vlan", 13: "bridge",
	14: "generic", 16: "tun", 29: "wireguard", 30: "wifi-p2p", 32: "loopback",
}

var nmDeviceStates = map[uint32]string{
	10: "unmanaged", 20: "unavailable", 30: "disconnected", 40: "prepare", 50: "config",
	60: "need-auth", 70: "ip-config", 80: "ip-check", 90: "secondaries", 100: "activated",
	110: "deactivating", 120: "failed",
}

func name(m map[uint32]string, v uint32) string {
	if s, ok := m[v]; ok {
		return s
	}
	return "unknown"
}

// Status returns the overview with every device and its addresses.
func (n *NetworkManager) Status(ctx context.Context) (NetworkStatus, error) {
	p, err := n.Bus.getAll(ctx, nmDest, nmPath, nmIface)
	if err != nil {
		return NetworkStatus{}, fmt.Errorf("NetworkManager: %w", err)
	}
	st := NetworkStatus{
		Version:           str(p, "Version"),
		State:             name(nmStates, u32(p, "State")),
		Connectivity:      name(nmConnectivity, u32(p, "Connectivity")),
		NetworkingEnabled: boolean(p, "NetworkingEnabled"),
		WirelessEnabled:   boolean(p, "WirelessEnabled"),
		WirelessHardware:  boolean(p, "WirelessHardwareEnabled"),
		Devices:           []NetworkDevice{},
	}
	for _, dp := range objPaths(p, "Devices") {
		d, err := n.device(ctx, dp)
		if err != nil {
			return NetworkStatus{}, err
		}
		if d.Type == "loopback" {
			continue
		}
		st.Devices = append(st.Devices, d)
	}
	return st, nil
}

func (n *NetworkManager) device(ctx context.Context, path dbus.ObjectPath) (NetworkDevice, error) {
	p, err := n.Bus.getAll(ctx, nmDest, path, nmIface+".Device")
	if err != nil {
		return NetworkDevice{}, fmt.Errorf("device %s: %w", path, err)
	}
	d := NetworkDevice{
		Interface: str(p, "Interface"),
		Type:      name(nmDeviceTypes, u32(p, "DeviceType")),
		State:     name(nmDeviceStates, u32(p, "State")),
		HwAddress: str(p, "HwAddress"),
	}
	if ac := objPath(p, "ActiveConnection"); ac != "" && ac != "/" {
		if ap, err := n.Bus.getAll(ctx, nmDest, ac, nmIface+".Connection.Active"); err == nil {
			d.Connection = str(ap, "Id")
		}
	}
	if ip4 := objPath(p, "Ip4Config"); ip4 != "" && ip4 != "/" {
		if ip, err := n.Bus.getAll(ctx, nmDest, ip4, nmIface+".IP4Config"); err == nil {
			d.IPv4 = addressData(ip)
			d.Gateway4 = str(ip, "Gateway")
			if ns, ok := ip["NameserverData"].Value().([]map[string]dbus.Variant); ok {
				for _, e := range ns {
					if a := str(e, "address"); a != "" {
						d.DNS4 = append(d.DNS4, a)
					}
				}
			}
		}
	}
	if ip6 := objPath(p, "Ip6Config"); ip6 != "" && ip6 != "/" {
		if ip, err := n.Bus.getAll(ctx, nmDest, ip6, nmIface+".IP6Config"); err == nil {
			d.IPv6 = addressData(ip)
		}
	}
	return d, nil
}

func addressData(p map[string]dbus.Variant) []string {
	var out []string
	list, _ := p["AddressData"].Value().([]map[string]dbus.Variant)
	for _, a := range list {
		addr := str(a, "address")
		if addr == "" {
			continue
		}
		out = append(out, fmt.Sprintf("%s/%d", addr, u32(a, "prefix")))
	}
	return out
}

// Connections lists saved profiles and marks the active ones.
func (n *NetworkManager) Connections(ctx context.Context) ([]Connection, error) {
	var paths []dbus.ObjectPath
	if err := n.Bus.call(ctx, nmDest, nmSettings, nmIface+".Settings.ListConnections", []any{&paths}); err != nil {
		return nil, fmt.Errorf("list connections: %w", err)
	}
	active, err := n.active(ctx)
	if err != nil {
		return nil, err
	}
	out := make([]Connection, 0, len(paths))
	for _, p := range paths {
		var s map[string]map[string]dbus.Variant
		if err := n.Bus.call(ctx, nmDest, p, nmIface+".Settings.Connection.GetSettings", []any{&s}); err != nil {
			return nil, fmt.Errorf("connection %s: %w", p, err)
		}
		c := Connection{
			UUID:        str(s["connection"], "uuid"),
			ID:          str(s["connection"], "id"),
			Type:        str(s["connection"], "type"),
			Autoconnect: true,
			IPv4Method:  str(s["ipv4"], "method"),
		}
		if v, ok := s["connection"]["autoconnect"].Value().(bool); ok {
			c.Autoconnect = v
		}
		if a, ok := active[c.UUID]; ok {
			c.Active = true
			c.Device = a.device
		}
		out = append(out, c)
	}
	return out, nil
}

type activeConn struct {
	path   dbus.ObjectPath
	device string
}

func (n *NetworkManager) active(ctx context.Context) (map[string]activeConn, error) {
	p, err := n.Bus.getAll(ctx, nmDest, nmPath, nmIface)
	if err != nil {
		return nil, fmt.Errorf("NetworkManager: %w", err)
	}
	out := map[string]activeConn{}
	for _, ap := range objPaths(p, "ActiveConnections") {
		a, err := n.Bus.getAll(ctx, nmDest, ap, nmIface+".Connection.Active")
		if err != nil {
			continue // went away meanwhile
		}
		ac := activeConn{path: ap}
		if devs := objPaths(a, "Devices"); len(devs) > 0 {
			if dp, err := n.Bus.getAll(ctx, nmDest, devs[0], nmIface+".Device"); err == nil {
				ac.device = str(dp, "Interface")
			}
		}
		out[str(a, "Uuid")] = ac
	}
	return out, nil
}

func (n *NetworkManager) connectionPath(ctx context.Context, uuid string) (dbus.ObjectPath, error) {
	var p dbus.ObjectPath
	if err := n.Bus.call(ctx, nmDest, nmSettings, nmIface+".Settings.GetConnectionByUuid", []any{&p}, uuid); err != nil {
		return "", fmt.Errorf("connection %s: %w", uuid, err)
	}
	return p, nil
}

// Up activates a saved connection (NetworkManager picks the device).
func (n *NetworkManager) Up(ctx context.Context, uuid string) error {
	p, err := n.connectionPath(ctx, uuid)
	if err != nil {
		return err
	}
	var active dbus.ObjectPath
	if err := n.Bus.call(ctx, nmDest, nmPath, nmIface+".ActivateConnection", []any{&active}, p, dbus.ObjectPath("/"), dbus.ObjectPath("/")); err != nil {
		return fmt.Errorf("activate %s: %w", uuid, err)
	}
	return nil
}

// Down deactivates an active connection.
func (n *NetworkManager) Down(ctx context.Context, uuid string) error {
	if _, err := n.connectionPath(ctx, uuid); err != nil {
		return err
	}
	active, err := n.active(ctx)
	if err != nil {
		return err
	}
	a, ok := active[uuid]
	if !ok {
		return errf(ErrConflict, "connection %s is not active", uuid)
	}
	if err := n.Bus.call(ctx, nmDest, nmPath, nmIface+".DeactivateConnection", nil, a.path); err != nil {
		return fmt.Errorf("deactivate %s: %w", uuid, err)
	}
	return nil
}

// SetWireless turns Wi-Fi on or off.
func (n *NetworkManager) SetWireless(ctx context.Context, enabled bool) error {
	if err := n.Bus.setProp(ctx, nmDest, nmPath, nmIface, "WirelessEnabled", enabled); err != nil {
		return fmt.Errorf("wireless: %w", err)
	}
	return nil
}
