// SPDX-License-Identifier: MPL-2.0

package fakebus

import (
	"github.com/godbus/dbus/v5"
)

// Fixture of the fake UDisks2:
//
//	nvme0n1  NVMe system disk: p1 vfat /boot/efi, p2 btrfs (RootUUID) /sysroot
//	sda      SATA HDD with 8 bad sectors: sda1 btrfs "media" (MediaUUID), not mounted
//	sdb      USB stick, SMART failing: sdb1 exfat "USB" mounted under /run/media
const (
	udIface = "org.freedesktop.UDisks2"
	udPath  = dbus.ObjectPath("/org/freedesktop/UDisks2")

	RootUUID  = "1f0e2d3c-4b5a-4968-8776-655443322110"
	MediaUUID = "9a8b7c6d-5e4f-4a3b-8c2d-1e0f9a8b7c6d"

	drvNVMe = dbus.ObjectPath("/org/freedesktop/UDisks2/drives/Samsung_SSD_980_S1")
	drvHDD  = dbus.ObjectPath("/org/freedesktop/UDisks2/drives/WDC_WD40EFRX_W1")
	drvUSB  = dbus.ObjectPath("/org/freedesktop/UDisks2/drives/SanDisk_Ultra_U1")
	blkBase = "/org/freedesktop/UDisks2/block_devices/"
)

// BlockPath is the object path of a block device ("sda1").
func BlockPath(dev string) dbus.ObjectPath { return dbus.ObjectPath(blkBase + dev) }

func cstr(s string) []byte { return append([]byte(s), 0) }

func mounts(mps ...string) [][]byte {
	out := [][]byte{}
	for _, m := range mps {
		out = append(out, cstr(m))
	}
	return out
}

type ifaces = map[string]map[string]dbus.Variant

func (f *Fake) udisks() error {
	objs := map[dbus.ObjectPath]map[string]map[string]any{
		drvNVMe: {
			udIface + ".Drive": {"Id": "Samsung-SSD-980-S1", "Vendor": "", "Model": "Samsung SSD 980 500GB", "Serial": "S1XXXX",
				"Size": uint64(500107862016), "ConnectionBus": "", "Removable": false, "RotationRate": int32(0)},
			udIface + ".NVMe.Controller": {"SmartUpdated": uint64(1795593600), "SmartTemperature": uint16(311),
				"SmartPowerOnHours": uint64(4211), "SmartCritical": []string{}},
		},
		drvHDD: {
			udIface + ".Drive": {"Id": "WDC-WD40EFRX-W1", "Vendor": "WDC", "Model": "WD40EFRX", "Serial": "WD-W1XXXX",
				"Size": uint64(4000787030016), "ConnectionBus": "", "Removable": false, "RotationRate": int32(5400)},
			udIface + ".Drive.Ata": {"SmartSupported": true, "SmartEnabled": true, "SmartUpdated": uint64(1795593600),
				"SmartFailing": false, "SmartTemperature": float64(308.15), "SmartPowerOnSeconds": uint64(36000 * 3600),
				"SmartNumAttributesFailing": int32(0), "SmartNumBadSectors": int64(8), "SmartSelftestStatus": "success"},
		},
		drvUSB: {
			udIface + ".Drive": {"Id": "SanDisk-Ultra-U1", "Vendor": "SanDisk", "Model": "Ultra", "Serial": "U1XXXX",
				"Size": uint64(64023257088), "ConnectionBus": "usb", "Removable": true, "RotationRate": int32(0)},
			udIface + ".Drive.Ata": {"SmartSupported": true, "SmartEnabled": true, "SmartUpdated": uint64(1795593600),
				"SmartFailing": true, "SmartTemperature": float64(0), "SmartPowerOnSeconds": uint64(0),
				"SmartNumAttributesFailing": int32(1), "SmartNumBadSectors": int64(-1), "SmartSelftestStatus": "success"},
		},
	}
	block := func(dev string, drive dbus.ObjectPath, size uint64, usage, fsType, label, uuid string) map[string]any {
		return map[string]any{
			"Device": cstr("/dev/" + dev), "Drive": drive, "Size": size, "IdUsage": usage, "IdType": fsType,
			"IdLabel": label, "IdUUID": uuid, "ReadOnly": false, "HintIgnore": false, "HintSystem": !(drive == drvUSB),
		}
	}
	objs[BlockPath("nvme0n1")] = map[string]map[string]any{udIface + ".Block": block("nvme0n1", drvNVMe, 500107862016, "", "", "", "")}
	objs[BlockPath("nvme0n1p1")] = map[string]map[string]any{
		udIface + ".Block":      block("nvme0n1p1", drvNVMe, 629145600, "filesystem", "vfat", "EFI-SYSTEM", "A1B2-C3D4"),
		udIface + ".Filesystem": {"MountPoints": mounts("/boot/efi")},
	}
	objs[BlockPath("nvme0n1p2")] = map[string]map[string]any{
		udIface + ".Block":      block("nvme0n1p2", drvNVMe, 499000000000, "filesystem", "btrfs", "root", RootUUID),
		udIface + ".Filesystem": {"MountPoints": mounts("/sysroot", "/var")},
	}
	objs[BlockPath("sda")] = map[string]map[string]any{udIface + ".Block": block("sda", drvHDD, 4000787030016, "", "", "", "")}
	objs[BlockPath("sda1")] = map[string]map[string]any{
		udIface + ".Block":      block("sda1", drvHDD, 4000785104896, "filesystem", "btrfs", "media", MediaUUID),
		udIface + ".Filesystem": {"MountPoints": mounts()},
	}
	objs[BlockPath("sdb")] = map[string]map[string]any{udIface + ".Block": block("sdb", drvUSB, 64023257088, "", "", "", "")}
	objs[BlockPath("sdb1")] = map[string]map[string]any{
		udIface + ".Block":      block("sdb1", drvUSB, 64022208512, "filesystem", "exfat", "USB", "1234-ABCD"),
		udIface + ".Filesystem": {"MountPoints": mounts("/run/media/alpha-admin/USB")},
	}
	objs[BlockPath("loop0")] = map[string]map[string]any{udIface + ".Block": block("loop0", "/", 1<<20, "filesystem", "squashfs", "", "")}

	for path, ifs := range objs {
		for iface, props := range ifs {
			if err := f.object(path, iface, props); err != nil {
				return err
			}
		}
		if _, ok := ifs[udIface+".Filesystem"]; ok {
			path := path
			if err := f.conn.ExportMethodTable(map[string]any{
				"Mount": func(opts map[string]dbus.Variant) (string, *dbus.Error) {
					if err := f.record("Mount", path); err != nil {
						return "", err
					}
					label := f.Prop(path, udIface+".Block", "IdLabel").(string)
					where := "/run/media/alpha-admin/" + label
					f.SetProp(path, udIface+".Filesystem", "MountPoints", mounts(where))
					return where, nil
				},
				"Unmount": func(opts map[string]dbus.Variant) *dbus.Error {
					if err := f.record("Unmount", path); err != nil {
						return err
					}
					f.SetProp(path, udIface+".Filesystem", "MountPoints", mounts())
					return nil
				},
			}, path, udIface+".Filesystem"); err != nil {
				return err
			}
		}
	}
	keys := make([]dbus.ObjectPath, 0, len(objs))
	for p := range objs {
		keys = append(keys, p)
	}
	return f.conn.ExportMethodTable(map[string]any{
		"GetManagedObjects": func() (map[dbus.ObjectPath]ifaces, *dbus.Error) {
			f.mu.Lock()
			defer f.mu.Unlock()
			out := map[dbus.ObjectPath]ifaces{}
			for _, p := range keys {
				obj := ifaces{}
				for iface, props := range f.props[p] {
					if iface == "org.freedesktop.DBus.Properties" {
						continue
					}
					obj[iface] = map[string]dbus.Variant{}
					for k, v := range props {
						obj[iface][k] = v
					}
				}
				out[p] = obj
			}
			return out, nil
		},
	}, udPath, "org.freedesktop.DBus.ObjectManager")
}
