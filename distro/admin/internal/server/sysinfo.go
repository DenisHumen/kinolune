// SPDX-License-Identifier: MPL-2.0

package server

import (
	"bufio"
	"encoding/json"
	"errors"
	"net/http"
	"os"
	"path/filepath"
	"strconv"
	"strings"

	"github.com/DenisHumen/kinolune/distro/admin/internal/auth"
)

// InstallInfoPath is written by the installer (DIST-005).
const InstallInfoPath = "/var/lib/alpha/install.json"

// InstallInfo is the part of install.json the admin backend uses.
type InstallInfo struct {
	Schema        int               `json:"schema"`
	InstallID     string            `json:"install_id"`
	InstalledAt   string            `json:"installed_at"`
	Hostname      string            `json:"hostname"`
	Language      string            `json:"language"`
	AdminUser     string            `json:"admin_user"`
	UpdateChannel string            `json:"update_channel"`
	FileSystems   map[string]string `json:"filesystems"` // label → UUID
	DataPool      *struct {
		Name       string `json:"name"`
		Mountpoint string `json:"mountpoint"`
		UUID       string `json:"uuid"`
	} `json:"data_pool"`
}

// LoadInstallInfo reads install.json; a missing file is not an error
// (nil, nil): machines installed before DIST-005 have none.
func LoadInstallInfo(path string) (*InstallInfo, error) {
	b, err := os.ReadFile(path)
	if errors.Is(err, os.ErrNotExist) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	var info InstallInfo
	if err := json.Unmarshal(b, &info); err != nil {
		return nil, err
	}
	return &info, nil
}

// SystemUUIDs are the OS file systems from install.json (all but the data
// pool): the admin UI never mounts or unmounts them.
func (i *InstallInfo) SystemUUIDs() []string {
	if i == nil {
		return nil
	}
	var out []string
	for _, uuid := range i.FileSystems {
		if uuid == "" || (i.DataPool != nil && uuid == i.DataPool.UUID) {
			continue
		}
		out = append(out, uuid)
	}
	return out
}

// ReadOSVersion reads ALPHA_OS_VERSION from /usr/share/alpha/version.
func ReadOSVersion(path string) string {
	f, err := os.Open(path)
	if err != nil {
		return "unknown"
	}
	defer f.Close()
	sc := bufio.NewScanner(f)
	for sc.Scan() {
		if k, v, ok := strings.Cut(strings.TrimSpace(sc.Text()), "="); ok && k == "ALPHA_OS_VERSION" {
			return v
		}
	}
	return "unknown"
}

type systemResponse struct {
	Hostname       string       `json:"hostname"`
	OSVersion      string       `json:"os_version"`
	Kernel         string       `json:"kernel,omitempty"`
	UptimeSeconds  int64        `json:"uptime_seconds"`
	Load           [3]float64   `json:"load"`
	CPUs           int          `json:"cpus"`
	MemoryTotal    uint64       `json:"memory_total"`
	MemoryAvail    uint64       `json:"memory_available"`
	TLSFingerprint string       `json:"tls_fingerprint"`
	Install        *installView `json:"install,omitempty"`
}

type installView struct {
	InstalledAt   string `json:"installed_at,omitempty"`
	Language      string `json:"language,omitempty"`
	UpdateChannel string `json:"update_channel,omitempty"`
	DataPool      string `json:"data_pool,omitempty"`
	DataPoolPath  string `json:"data_pool_path,omitempty"`
}

func (s *Server) systemInfo(w http.ResponseWriter, r *http.Request, _ *auth.Session) {
	proc := s.Info.ProcRoot
	if proc == "" {
		proc = "/proc"
	}
	resp := systemResponse{OSVersion: s.Info.OSVersion, TLSFingerprint: s.Info.TLSFingerprint}
	hn := s.Info.Hostname
	if hn == nil {
		hn = os.Hostname
	}
	resp.Hostname, _ = hn()
	if b, err := os.ReadFile(filepath.Join(proc, "sys/kernel/osrelease")); err == nil {
		resp.Kernel = strings.TrimSpace(string(b))
	}
	if b, err := os.ReadFile(filepath.Join(proc, "uptime")); err == nil {
		if f := strings.Fields(string(b)); len(f) > 0 {
			up, _ := strconv.ParseFloat(f[0], 64)
			resp.UptimeSeconds = int64(up)
		}
	}
	if b, err := os.ReadFile(filepath.Join(proc, "loadavg")); err == nil {
		f := strings.Fields(string(b))
		for i := 0; i < 3 && i < len(f); i++ {
			resp.Load[i], _ = strconv.ParseFloat(f[i], 64)
		}
	}
	if b, err := os.ReadFile(filepath.Join(proc, "meminfo")); err == nil {
		for _, line := range strings.Split(string(b), "\n") {
			f := strings.Fields(line)
			if len(f) < 2 {
				continue
			}
			kb, _ := strconv.ParseUint(f[1], 10, 64)
			switch f[0] {
			case "MemTotal:":
				resp.MemoryTotal = kb * 1024
			case "MemAvailable:":
				resp.MemoryAvail = kb * 1024
			}
		}
	}
	if b, err := os.ReadFile(filepath.Join(proc, "cpuinfo")); err == nil {
		resp.CPUs = strings.Count(string(b), "\nprocessor") + boolInt(strings.HasPrefix(string(b), "processor"))
	}
	if i := s.Info.Install; i != nil {
		v := &installView{InstalledAt: i.InstalledAt, Language: i.Language, UpdateChannel: i.UpdateChannel}
		if i.DataPool != nil {
			v.DataPool, v.DataPoolPath = i.DataPool.Name, i.DataPool.Mountpoint
		}
		resp.Install = v
	}
	writeJSON(w, http.StatusOK, resp)
}

func boolInt(b bool) int {
	if b {
		return 1
	}
	return 0
}
