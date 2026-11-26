// SPDX-License-Identifier: MPL-2.0

package system

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/DenisHumen/kinolune/distro/admin/internal/system/fakebus"
)

func newBus(t *testing.T) (*fakebus.Fake, *Bus) {
	t.Helper()
	f := fakebus.Start(t)
	return f, &Bus{Conn: f.Client(t), Timeout: 5 * time.Second}
}

var ctx = context.Background()

func TestSystemdUnits(t *testing.T) {
	_, bus := newBus(t)
	sd := &Systemd{Bus: bus}
	units, err := sd.Units(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if len(units) != len(OurUnits) {
		t.Fatalf("%d units", len(units))
	}
	byName := map[string]UnitStatus{}
	for _, u := range units {
		byName[u.Name] = u
	}
	media := byName["alpha-media.service"]
	if media.ActiveState != "active" || media.SubState != "running" || media.MainPID != 1234 || media.Since == nil || !media.Restartable {
		t.Errorf("media: %+v", media)
	}
	if smb := byName["smb.service"]; smb.ActiveState != "failed" || smb.Result != "exit-code" {
		t.Errorf("smb: %+v", smb)
	}
	if c := byName["cockpit.socket"]; c.ActiveState != "inactive" || c.UnitFileState != "disabled" || c.Restartable {
		t.Errorf("cockpit: %+v", c)
	}
	if _, err := sd.Unit(ctx, "getty@tty1.service"); !errors.Is(err, ErrNotFound) {
		t.Errorf("foreign unit: %v", err)
	}
}

func TestSystemdRestart(t *testing.T) {
	f, bus := newBus(t)
	sd := &Systemd{Bus: bus}
	job, err := sd.Restart(ctx, "smb.service")
	if err != nil || !strings.HasPrefix(job, "/org/freedesktop/systemd1/job/") {
		t.Fatalf("restart: %q %v", job, err)
	}
	if u, _ := sd.Unit(ctx, "smb.service"); u.ActiveState != "active" {
		t.Errorf("after restart: %+v", u)
	}
	if _, err := sd.Restart(ctx, "sshd.service"); !errors.Is(err, ErrForbidden) {
		t.Errorf("sshd restart: %v", err)
	}
	if _, err := sd.Restart(ctx, "getty@tty1.service"); !errors.Is(err, ErrNotFound) {
		t.Errorf("foreign restart: %v", err)
	}
	f.Deny("RestartUnit")
	if _, err := sd.Restart(ctx, "alpha-media.service"); !errors.Is(err, ErrForbidden) {
		t.Errorf("polkit denial: %v", err)
	}
	if got := f.Calls(); !slices.Equal(got, []string{"RestartUnit smb.service replace"}) {
		t.Errorf("calls %q", got)
	}
}

func TestJournal(t *testing.T) {
	dir := t.TempDir()
	script := filepath.Join(dir, "journalctl")
	// A fake journalctl: checks the arguments and prints two JSON entries,
	// one with a binary message.
	body := `#!/bin/sh
[ "$1" = "--unit=alpha-media.service" ] || { echo "bad unit $1" >&2; exit 2; }
[ "$2" = "--lines=50" ] || { echo "bad lines $2" >&2; exit 2; }
echo '{"__REALTIME_TIMESTAMP":"1795593600000000","PRIORITY":"6","_PID":"1234","MESSAGE":"server started"}'
echo '{"__REALTIME_TIMESTAMP":"1795593601000000","PRIORITY":"3","MESSAGE":[104,105,255]}'
`
	if err := os.WriteFile(script, []byte(body), 0o755); err != nil {
		t.Fatal(err)
	}
	sd := &Systemd{Journalctl: script}
	entries, err := sd.Journal(ctx, "alpha-media.service", 50)
	if err != nil {
		t.Fatal(err)
	}
	if len(entries) != 2 || entries[0].Message != "server started" || entries[0].PID != 1234 || entries[0].Priority != 6 {
		t.Fatalf("%+v", entries)
	}
	if entries[1].Message != "hi�" || entries[1].Priority != 3 || entries[1].Time.Unix() != 1795593601 {
		t.Fatalf("%+v", entries[1])
	}
	if _, err := sd.Journal(ctx, "../../etc/passwd", 10); !errors.Is(err, ErrNotFound) {
		t.Fatalf("foreign unit journal: %v", err)
	}
	if _, err := sd.Journal(ctx, "smb.service", 10); err == nil {
		t.Fatal("journalctl failure must be reported")
	}
}

func TestNetworkStatus(t *testing.T) {
	_, bus := newBus(t)
	nm := &NetworkManager{Bus: bus}
	st, err := nm.Status(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if st.State != "connected-global" || st.Connectivity != "full" || !st.WirelessEnabled || st.Version == "" {
		t.Errorf("%+v", st)
	}
	if len(st.Devices) != 2 {
		t.Fatalf("loopback must be hidden: %+v", st.Devices)
	}
	eth := st.Devices[0]
	if eth.Interface != "enp1s0" || eth.Type != "ethernet" || eth.State != "activated" ||
		!slices.Equal(eth.IPv4, []string{"192.168.1.20/24"}) || eth.Gateway4 != "192.168.1.1" ||
		!slices.Equal(eth.DNS4, []string{"192.168.1.1"}) || eth.Connection != "Проводное подключение 1" {
		t.Errorf("eth: %+v", eth)
	}
	if w := st.Devices[1]; w.Type != "wifi" || w.State != "disconnected" {
		t.Errorf("wifi: %+v", w)
	}
}

func TestNetworkConnections(t *testing.T) {
	f, bus := newBus(t)
	nm := &NetworkManager{Bus: bus}
	conns, err := nm.Connections(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if len(conns) != 2 || !conns[0].Active || conns[0].Device != "enp1s0" || conns[1].Active || conns[1].Autoconnect {
		t.Fatalf("%+v", conns)
	}
	if err := nm.Up(ctx, fakebus.WifiUUID); err != nil {
		t.Fatal(err)
	}
	conns, _ = nm.Connections(ctx)
	if !conns[1].Active || conns[1].Device != "wlp2s0" {
		t.Fatalf("after up: %+v", conns[1])
	}
	if err := nm.Down(ctx, fakebus.WifiUUID); err != nil {
		t.Fatal(err)
	}
	if err := nm.Down(ctx, fakebus.WifiUUID); !errors.Is(err, ErrConflict) {
		t.Fatalf("down twice: %v", err)
	}
	if err := nm.Up(ctx, "00000000-0000-0000-0000-000000000000"); !errors.Is(err, ErrNotFound) {
		t.Fatalf("unknown uuid: %v", err)
	}
	if err := nm.SetWireless(ctx, false); err != nil {
		t.Fatal(err)
	}
	if st, _ := nm.Status(ctx); st.WirelessEnabled {
		t.Fatal("wifi still enabled")
	}
	f.Deny("ActivateConnection")
	if err := nm.Up(ctx, fakebus.WifiUUID); !errors.Is(err, ErrForbidden) {
		t.Fatalf("denied: %v", err)
	}
}

func TestFirewall(t *testing.T) {
	f, bus := newBus(t)
	fw := &Firewalld{Bus: bus}
	st, err := fw.Status(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if !st.Running || st.DefaultZone != "home" || len(st.Zones) != 1 || !slices.Equal(st.Zones[0].Interfaces, []string{"enp1s0"}) {
		t.Fatalf("%+v", st)
	}
	open := map[string]bool{}
	for _, o := range st.Openings {
		open[o.Name] = o.Open
	}
	if !open["media"] || !open["smb"] || !open["discovery"] || open["cockpit"] || open["ssh"] {
		t.Fatalf("openings %v", open)
	}
	// Close the media port, open Cockpit: runtime and permanent both change.
	if err := fw.SetOpening(ctx, "media", false); err != nil {
		t.Fatal(err)
	}
	if err := fw.SetOpening(ctx, "cockpit", true); err != nil {
		t.Fatal(err)
	}
	rtS, rtP, pS, pP := f.Zone("home")
	if slices.Contains(rtP, "8487/tcp") || slices.Contains(pP, "8487/tcp") {
		t.Errorf("port still open: %v %v", rtP, pP)
	}
	if !slices.Contains(rtS, "cockpit") || !slices.Contains(pS, "cockpit") {
		t.Errorf("cockpit not open: %v %v", rtS, pS)
	}
	// Idempotent.
	if err := fw.SetOpening(ctx, "cockpit", true); err != nil {
		t.Fatalf("second open: %v", err)
	}
	if err := fw.SetOpening(ctx, "telnet", true); !errors.Is(err, ErrNotFound) {
		t.Fatalf("unknown opening: %v", err)
	}
	// SSH (DIST-018): closed by default, opens as the ssh service.
	if err := fw.SetOpening(ctx, "ssh", true); err != nil {
		t.Fatal(err)
	}
	if rtS, _, pS, _ := f.Zone("home"); !slices.Contains(rtS, "ssh") || !slices.Contains(pS, "ssh") {
		t.Errorf("ssh not open: %v %v", rtS, pS)
	}
	// SMB is 445/tcp only; an older image's "samba" service (NetBIOS
	// 137-139 too) counts as open and goes away on close.
	if err := fw.SetOpening(ctx, "smb", false); err != nil {
		t.Fatal(err)
	}
	var ignored string
	if err := bus.call(ctx, fwDest, fwPath, fwIface+".zone.addService", []any{&ignored}, "home", "samba", int32(0)); err != nil {
		t.Fatal(err)
	}
	if st, _ := fw.Status(ctx); !openingOpen(st, "smb") {
		t.Errorf("legacy samba service not seen as smb open")
	}
	if err := fw.SetOpening(ctx, "smb", false); err != nil {
		t.Fatal(err)
	}
	if rtS, rtP, _, pP := f.Zone("home"); slices.Contains(rtS, "samba") || slices.Contains(rtP, "445/tcp") || slices.Contains(pP, "445/tcp") {
		t.Errorf("smb still open: %v %v %v", rtS, rtP, pP)
	}
	if err := fw.SetOpening(ctx, "smb", true); err != nil {
		t.Fatal(err)
	}
	if rtS, rtP, pS, pP := f.Zone("home"); slices.Contains(rtS, "samba") || slices.Contains(pS, "samba") ||
		!slices.Contains(rtP, "445/tcp") || !slices.Contains(pP, "445/tcp") {
		t.Errorf("smb open: %v %v %v %v", rtS, rtP, pS, pP)
	}
	f.Deny("runtime.removePort")
	if err := fw.SetOpening(ctx, "smb", false); !errors.Is(err, ErrForbidden) {
		t.Fatalf("denied: %v", err)
	}
}

func openingOpen(st FirewallStatus, name string) bool {
	for _, o := range st.Openings {
		if o.Name == name {
			return o.Open
		}
	}
	return false
}

func TestDisks(t *testing.T) {
	f, bus := newBus(t)
	ud := &UDisks{Bus: bus, SystemUUIDs: []string{fakebus.RootUUID}}
	rep, err := ud.List(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if len(rep.Drives) != 3 {
		t.Fatalf("%+v", rep.Drives)
	}
	by := map[string]Drive{}
	for _, d := range rep.Drives {
		by[d.Model] = d
	}
	nvme := by["Samsung SSD 980 500GB"]
	if !nvme.System || nvme.Health.Status != "ok" || nvme.Health.TemperatureC == nil || int(*nvme.Health.TemperatureC) != 37 || *nvme.Health.PowerOnHours != 4211 {
		t.Errorf("nvme: %+v %+v", nvme, nvme.Health)
	}
	for _, b := range nvme.Blocks {
		if b.Mountable || b.Unmountable {
			t.Errorf("system block offered: %+v", b)
		}
	}
	hdd := by["WD40EFRX"]
	if hdd.System || hdd.Health.Status != "warning" || *hdd.Health.BadSectors != 8 || hdd.Rotation != 5400 || *hdd.Health.PowerOnHours != 36000 {
		t.Errorf("hdd: %+v %+v", hdd, hdd.Health)
	}
	if len(hdd.Blocks) != 2 || !hdd.Blocks[1].Mountable || hdd.Blocks[1].Label != "media" {
		t.Errorf("hdd blocks: %+v", hdd.Blocks)
	}
	usb := by["Ultra"]
	if usb.Health.Status != "failing" || !usb.Removable || usb.Bus != "usb" || !usb.Blocks[1].Unmountable {
		t.Errorf("usb: %+v", usb)
	}
	for _, d := range rep.Drives {
		for _, b := range d.Blocks {
			if strings.HasPrefix(b.Device, "/dev/loop") {
				t.Error("loop device listed")
			}
		}
	}

	where, err := ud.Mount(ctx, "sda1")
	if err != nil || where != "/run/media/alpha-admin/media" {
		t.Fatalf("mount: %q %v", where, err)
	}
	if _, err := ud.Mount(ctx, "/dev/sda1"); !errors.Is(err, ErrConflict) {
		t.Fatalf("mount twice: %v", err)
	}
	if err := ud.Unmount(ctx, "sda1"); err != nil {
		t.Fatal(err)
	}
	if _, err := ud.Mount(ctx, "nvme0n1p2"); !errors.Is(err, ErrConflict) {
		t.Fatalf("mounted system fs: %v", err)
	}
	if err := ud.Unmount(ctx, "nvme0n1p2"); !errors.Is(err, ErrForbidden) {
		t.Fatalf("unmount root: %v", err)
	}
	if err := ud.Unmount(ctx, "nvme0n1p1"); !errors.Is(err, ErrForbidden) {
		t.Fatalf("unmount ESP: %v", err)
	}
	if _, err := ud.Mount(ctx, "sdz9"); !errors.Is(err, ErrNotFound) {
		t.Fatalf("unknown device: %v", err)
	}
	// A system file system that happens to be unmounted is still refused.
	f.SetProp(fakebus.BlockPath("nvme0n1p2"), "org.freedesktop.UDisks2.Filesystem", "MountPoints", [][]byte{})
	if _, err := ud.Mount(ctx, "nvme0n1p2"); !errors.Is(err, ErrForbidden) {
		t.Fatalf("system fs by uuid: %v", err)
	}
	if got := f.Calls(); len(got) != 2 {
		t.Fatalf("calls %q", got)
	}
}

func TestPower(t *testing.T) {
	f, bus := newBus(t)
	l := &Logind{Bus: bus}
	inh, err := l.Inhibitors(ctx)
	if err != nil || len(inh) != 1 || inh[0].Who != "alpha-media" || inh[0].Mode != "delay" {
		t.Fatalf("%+v %v", inh, err)
	}
	if err := l.Reboot(ctx); err != nil {
		t.Fatal(err)
	}
	f.Deny("PowerOff")
	if err := l.PowerOff(ctx); !errors.Is(err, ErrForbidden) {
		t.Fatalf("denied: %v", err)
	}
	if got := f.Calls(); !slices.Equal(got, []string{"Reboot false"}) {
		t.Fatalf("calls %q", got)
	}
}
