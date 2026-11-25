// SPDX-License-Identifier: MPL-2.0

package system

import (
	"context"
	"fmt"
	"path"
	"slices"
	"sort"
	"strings"

	"github.com/godbus/dbus/v5"
)

const (
	udDest  = "org.freedesktop.UDisks2"
	udPath  = dbus.ObjectPath("/org/freedesktop/UDisks2")
	udIface = "org.freedesktop.UDisks2"
)

// Drive is a physical disk with its health summary.
type Drive struct {
	ID        string   `json:"id"`
	Vendor    string   `json:"vendor,omitempty"`
	Model     string   `json:"model"`
	Serial    string   `json:"serial,omitempty"`
	Size      uint64   `json:"size"`
	Bus       string   `json:"bus,omitempty"` // usb, sdio, ata (empty), nvme…
	Removable bool     `json:"removable"`
	Rotation  int32    `json:"rotation_rpm"` // 0 = SSD, -1 = rotating, unknown speed
	Health    Health   `json:"health"`
	Blocks    []Block  `json:"blocks"`
	System    bool     `json:"system"` // holds the running OS
	Devices   []string `json:"devices,omitempty"`
}

// Health is the SMART summary of a drive.
type Health struct {
	// Status: ok | warning | failing | unknown (no SMART data).
	Status         string   `json:"status"`
	SmartSupported bool     `json:"smart_supported"`
	SmartEnabled   bool     `json:"smart_enabled"`
	TemperatureC   *float64 `json:"temperature_c,omitempty"`
	PowerOnHours   *uint64  `json:"power_on_hours,omitempty"`
	BadSectors     *int64   `json:"bad_sectors,omitempty"`
	// Problems are short machine-readable reasons: "smart-failing",
	// "attributes-failing", "bad-sectors", "nvme-critical:<flag>", "selftest-failed".
	Problems []string `json:"problems,omitempty"`
}

// Block is a block device (whole disk, partition or md array) with its file
// system, if any.
type Block struct {
	Device      string   `json:"device"` // /dev/sda1
	Size        uint64   `json:"size"`
	Usage       string   `json:"usage,omitempty"` // filesystem | raid | crypto | other
	FSType      string   `json:"fs_type,omitempty"`
	Label       string   `json:"label,omitempty"`
	UUID        string   `json:"uuid,omitempty"`
	MountPoints []string `json:"mount_points,omitempty"`
	ReadOnly    bool     `json:"read_only,omitempty"`
	// Mountable: an unmounted data file system the admin may mount.
	Mountable bool `json:"mountable"`
	// Unmountable: mounted only under the data roots, may be unmounted.
	Unmountable bool `json:"unmountable"`

	path  dbus.ObjectPath
	drive dbus.ObjectPath
	isFS  bool
}

// DiskReport lists drives and block devices that belong to no drive
// (md arrays, loop devices are hidden).
type DiskReport struct {
	Drives []Drive  `json:"drives"`
	Other  []Block  `json:"other"`
	Notes  []string `json:"notes,omitempty"`
}

// systemMounts are mount points of the running OS: blocks mounted there are
// "system" and their file systems are never mounted/unmounted by the UI.
var systemMounts = []string{"/", "/sysroot", "/boot", "/boot/efi", "/usr", "/etc", "/var", "/var/home", "/home", "/root", "/opt"}

// DataRoots are where data pools live (05 §3, DIST-005: /var/srv/data/<pool>)
// and where UDisks mounts by default; only mounts below them can be undone.
var DataRoots = []string{"/var/srv", "/srv", "/run/media", "/media"}

// mountableFS are file systems a data pool or a USB disk may use.
var mountableFS = []string{"btrfs", "ext4", "ext3", "xfs", "exfat", "vfat", "ntfs", "ntfs3"}

// UDisks reads disks and mounts data file systems.
type UDisks struct {
	Bus *Bus
	// SystemUUIDs are file system UUIDs of the OS itself (from
	// /var/lib/alpha/install.json): never mounted or unmounted by the UI.
	SystemUUIDs []string
}

type managed map[dbus.ObjectPath]map[string]map[string]dbus.Variant

func (u *UDisks) objects(ctx context.Context) (managed, error) {
	var m managed
	if err := u.Bus.call(ctx, udDest, udPath, "org.freedesktop.DBus.ObjectManager.GetManagedObjects", []any{&m}); err != nil {
		return nil, fmt.Errorf("udisks: %w", err)
	}
	return m, nil
}

// List returns all drives with their blocks and SMART summary.
func (u *UDisks) List(ctx context.Context) (DiskReport, error) {
	m, err := u.objects(ctx)
	if err != nil {
		return DiskReport{}, err
	}
	blocks := u.blocks(m)
	rep := DiskReport{Drives: []Drive{}, Other: []Block{}}
	drives := map[dbus.ObjectPath]*Drive{}
	var order []dbus.ObjectPath
	for p, ifaces := range m {
		dp, ok := ifaces[udIface+".Drive"]
		if !ok {
			continue
		}
		d := &Drive{
			ID:        str(dp, "Id"),
			Vendor:    strings.TrimSpace(str(dp, "Vendor")),
			Model:     strings.TrimSpace(str(dp, "Model")),
			Serial:    str(dp, "Serial"),
			Size:      u64(dp, "Size"),
			Bus:       str(dp, "ConnectionBus"),
			Removable: boolean(dp, "Removable"),
			Rotation:  int32(i64(dp, "RotationRate")),
			Blocks:    []Block{},
		}
		d.Health = health(ifaces)
		drives[p] = d
		order = append(order, p)
	}
	for _, b := range blocks {
		if d, ok := drives[b.drive]; ok {
			d.Blocks = append(d.Blocks, b)
			for _, mp := range b.MountPoints {
				if slices.Contains(systemMounts, mp) {
					d.System = true
				}
			}
			if slices.Contains(u.SystemUUIDs, b.UUID) && b.UUID != "" {
				d.System = true
			}
		} else if strings.HasPrefix(b.Device, "/dev/md") || b.MountPoints != nil {
			rep.Other = append(rep.Other, b)
		}
	}
	sort.Slice(order, func(i, j int) bool { return order[i] < order[j] })
	for _, p := range order {
		d := drives[p]
		sort.Slice(d.Blocks, func(i, j int) bool { return d.Blocks[i].Device < d.Blocks[j].Device })
		for _, b := range d.Blocks {
			d.Devices = append(d.Devices, b.Device)
		}
		rep.Drives = append(rep.Drives, *d)
	}
	sort.Slice(rep.Other, func(i, j int) bool { return rep.Other[i].Device < rep.Other[j].Device })
	return rep, nil
}

func (u *UDisks) blocks(m managed) []Block {
	var out []Block
	for p, ifaces := range m {
		bp, ok := ifaces[udIface+".Block"]
		if !ok || boolean(bp, "HintIgnore") {
			continue
		}
		raw, _ := bp["Device"].Value().([]byte)
		dev := byteString(raw)
		if strings.HasPrefix(dev, "/dev/loop") || strings.HasPrefix(dev, "/dev/zram") || strings.HasPrefix(dev, "/dev/ram") {
			continue
		}
		b := Block{
			Device:   dev,
			Size:     u64(bp, "Size"),
			Usage:    str(bp, "IdUsage"),
			FSType:   str(bp, "IdType"),
			Label:    str(bp, "IdLabel"),
			UUID:     str(bp, "IdUUID"),
			ReadOnly: boolean(bp, "ReadOnly"),
			path:     p,
			drive:    objPath(bp, "Drive"),
		}
		if fs, ok := ifaces[udIface+".Filesystem"]; ok {
			b.isFS = true
			if mps, ok := fs["MountPoints"].Value().([][]byte); ok {
				for _, mp := range mps {
					b.MountPoints = append(b.MountPoints, byteString(mp))
				}
			}
		}
		b.Mountable = u.mountable(b)
		b.Unmountable = u.unmountable(b)
		out = append(out, b)
	}
	return out
}

func (u *UDisks) systemFS(b Block) bool {
	if b.UUID != "" && slices.Contains(u.SystemUUIDs, b.UUID) {
		return true
	}
	for _, mp := range b.MountPoints {
		if slices.Contains(systemMounts, mp) {
			return true
		}
	}
	return false
}

func (u *UDisks) mountable(b Block) bool {
	return b.isFS && len(b.MountPoints) == 0 && slices.Contains(mountableFS, b.FSType) && !u.systemFS(b)
}

func (u *UDisks) unmountable(b Block) bool {
	if !b.isFS || len(b.MountPoints) == 0 || u.systemFS(b) {
		return false
	}
	for _, mp := range b.MountPoints {
		if !underDataRoot(mp) {
			return false
		}
	}
	return true
}

func underDataRoot(mp string) bool {
	mp = path.Clean(mp)
	for _, r := range DataRoots {
		if strings.HasPrefix(mp, r+"/") {
			return true
		}
	}
	return false
}

func (u *UDisks) find(ctx context.Context, device string) (Block, error) {
	m, err := u.objects(ctx)
	if err != nil {
		return Block{}, err
	}
	if !strings.HasPrefix(device, "/dev/") {
		device = "/dev/" + device
	}
	for _, b := range u.blocks(m) {
		if b.Device == device {
			return b, nil
		}
	}
	return Block{}, errf(ErrNotFound, "block device %s", device)
}

// Mount mounts a data file system and returns where. UDisks uses the
// /etc/fstab entry when there is one (data pools from the installer),
// otherwise /run/media/<user>/<label>.
func (u *UDisks) Mount(ctx context.Context, device string) (string, error) {
	b, err := u.find(ctx, device)
	if err != nil {
		return "", err
	}
	if !b.Mountable {
		if len(b.MountPoints) > 0 {
			return "", errf(ErrConflict, "%s is already mounted", b.Device)
		}
		return "", errf(ErrForbidden, "%s is not a data file system", b.Device)
	}
	var where string
	if err := u.Bus.call(ctx, udDest, b.path, udIface+".Filesystem.Mount", []any{&where}, map[string]dbus.Variant{}); err != nil {
		return "", fmt.Errorf("mount %s: %w", b.Device, err)
	}
	return where, nil
}

// Unmount unmounts a data file system mounted under the data roots.
func (u *UDisks) Unmount(ctx context.Context, device string) error {
	b, err := u.find(ctx, device)
	if err != nil {
		return err
	}
	if len(b.MountPoints) == 0 {
		return errf(ErrConflict, "%s is not mounted", b.Device)
	}
	if !b.Unmountable {
		return errf(ErrForbidden, "%s is not a data file system", b.Device)
	}
	if err := u.Bus.call(ctx, udDest, b.path, udIface+".Filesystem.Unmount", nil, map[string]dbus.Variant{}); err != nil {
		return fmt.Errorf("unmount %s: %w", b.Device, err)
	}
	return nil
}

func health(ifaces map[string]map[string]dbus.Variant) Health {
	h := Health{Status: "unknown"}
	if ata, ok := ifaces[udIface+".Drive.Ata"]; ok {
		h.SmartSupported = boolean(ata, "SmartSupported")
		h.SmartEnabled = boolean(ata, "SmartEnabled")
		if !h.SmartSupported || !h.SmartEnabled || u64(ata, "SmartUpdated") == 0 {
			return h
		}
		h.Status = "ok"
		if k := f64(ata, "SmartTemperature"); k > 0 {
			c := k - 273.15
			h.TemperatureC = &c
		}
		if s := u64(ata, "SmartPowerOnSeconds"); s > 0 {
			hours := s / 3600
			h.PowerOnHours = &hours
		}
		bad := i64(ata, "SmartNumBadSectors")
		if bad >= 0 {
			h.BadSectors = &bad
		}
		if boolean(ata, "SmartFailing") {
			h.Problems = append(h.Problems, "smart-failing")
		}
		if i64(ata, "SmartNumAttributesFailing") > 0 {
			h.Problems = append(h.Problems, "attributes-failing")
		}
		if bad > 0 {
			h.Problems = append(h.Problems, "bad-sectors")
		}
		if s := str(ata, "SmartSelftestStatus"); s != "" && s != "success" && s != "inprogress" && s != "aborted" && s != "interrupted" {
			h.Problems = append(h.Problems, "selftest-failed")
		}
	} else if nv, ok := ifaces[udIface+".NVMe.Controller"]; ok {
		h.SmartSupported, h.SmartEnabled = true, true
		if u64(nv, "SmartUpdated") == 0 {
			h.Status = "unknown"
			return h
		}
		h.Status = "ok"
		if k := u64(nv, "SmartTemperature"); k > 0 {
			c := float64(k) - 273.15
			h.TemperatureC = &c
		}
		if hours := u64(nv, "SmartPowerOnHours"); hours > 0 {
			h.PowerOnHours = &hours
		}
		for _, f := range strs(nv, "SmartCritical") {
			h.Problems = append(h.Problems, "nvme-critical:"+f)
		}
	} else {
		return h
	}
	for _, p := range h.Problems {
		if p == "smart-failing" || strings.HasPrefix(p, "nvme-critical") || p == "selftest-failed" {
			h.Status = "failing"
			return h
		}
	}
	if len(h.Problems) > 0 {
		h.Status = "warning"
	} else if h.TemperatureC != nil && *h.TemperatureC >= 60 {
		h.Status = "warning"
		h.Problems = append(h.Problems, "hot")
	}
	return h
}
