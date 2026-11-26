// SPDX-License-Identifier: MPL-2.0

package webui

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"regexp"
	"slices"
	"sort"
	"strings"
	"testing"
)

// The embedded tokens.css must equal the generated one (go:embed cannot reach
// outside the module, so it is a copy).
func TestTokensUpToDate(t *testing.T) {
	gen, err := os.ReadFile(filepath.Join("..", "..", "..", "..", "design", "tokens", "generated", "web", "tokens.css"))
	if err != nil {
		t.Skipf("design tokens not found: %v", err)
	}
	got, _ := File("tokens.css")
	if string(got) != string(gen) {
		t.Fatal("static/tokens.css differs from design/tokens/generated/web/tokens.css: copy it again")
	}
	if !strings.Contains(string(gen), `[data-palette="bordeaux-graphite"]`) {
		t.Fatal("tokens.css has no bordeaux-graphite palette")
	}
}

// Colours only through semantic tokens, so the palette can be swapped.
func TestNoColorLiterals(t *testing.T) {
	css, _ := File("app.css")
	lit := regexp.MustCompile(`#[0-9a-fA-F]{3,8}\b|\b(rgba?|hsla?|oklch|color-mix)\(|:\s*(white|black)\b`)
	for i, line := range strings.Split(string(css), "\n") {
		if lit.MatchString(line) {
			t.Errorf("app.css:%d has a colour literal: %s", i+1, strings.TrimSpace(line))
		}
	}
	tokens, _ := File("tokens.css")
	for _, v := range regexp.MustCompile(`var\((--pa-[a-z0-9-]+)`).FindAllStringSubmatch(string(css), -1) {
		if !strings.Contains(string(tokens), v[1]+":") {
			t.Errorf("app.css uses %s, which tokens.css does not define", v[1])
		}
	}
}

// The CSP is default-src 'self': no inline scripts, styles or remote URLs.
func TestNoInlineCodeOrCDN(t *testing.T) {
	for _, name := range Names() {
		b, _ := File(name)
		s := string(b)
		if strings.HasSuffix(name, ".html") {
			if regexp.MustCompile(`<script(?:\s[^>]*)?>\s*[^<\s]`).MatchString(s) && !strings.Contains(s, "src=") {
				t.Errorf("%s: inline script", name)
			}
			if strings.Contains(s, "style=") || strings.Contains(s, "<style") {
				t.Errorf("%s: inline style", name)
			}
		}
		if strings.HasSuffix(name, ".js") || strings.HasSuffix(name, ".css") || strings.HasSuffix(name, ".html") {
			for _, m := range regexp.MustCompile(`(?:src|href|import|from|url\()\s*=?\s*['"(]?(https?:)?//[^'")\s]+`).FindAllString(s, -1) {
				if !strings.Contains(m, "github.com/DenisHumen/kinolune/releases") {
					t.Errorf("%s: remote resource %q", name, m)
				}
			}
			if strings.Contains(s, "setAttribute('style'") || strings.Contains(s, ".innerHTML = ") && name != "js/ui.js" {
				t.Errorf("%s: inline style attribute or innerHTML", name)
			}
		}
	}
}

func loadDict(t *testing.T, lang string) map[string]string {
	t.Helper()
	b, ok := File("i18n/" + lang + ".json")
	if !ok {
		t.Fatalf("i18n/%s.json is missing", lang)
	}
	var d map[string]string
	if err := json.Unmarshal(b, &d); err != nil {
		t.Fatalf("i18n/%s.json: %v", lang, err)
	}
	return d
}

// Every key the code uses exists in ru (the fallback), and en/uk have exactly
// the same keys and placeholders.
func TestI18nComplete(t *testing.T) {
	ru := loadDict(t, "ru")
	used := map[string]bool{}
	keyRe := regexp.MustCompile(`\bt\('([a-z0-9_.-]+)'|title: '([a-z0-9_.-]+)'|'((?:users)\.(?:grant|revoke)(?:_text)?)'`)
	for _, name := range Names() {
		if !strings.HasSuffix(name, ".js") {
			continue
		}
		b, _ := File(name)
		for _, m := range keyRe.FindAllStringSubmatch(string(b), -1) {
			for _, k := range m[1:] {
				if k != "" {
					used[k] = true
				}
			}
		}
	}
	// Keys built at run time from API values (openapi.yaml and alpha-updated enums).
	dyn := map[string][]string{
		"upd.state.":   {"up-to-date", "available", "downloaded", "staged", "rollback-queued", "unknown"},
		"upd.op.":      {"check", "download", "apply", "switch", "rollback"},
		"upd.channel.": {"stable", "beta"},
		"upd.auto.":    {"off", "download", "install"},
		"net.conn.":    {"none", "portal", "limited", "full", "unknown"},
		"disk.health.": {"ok", "warning", "failing", "unknown"},
		"fw.":          {"media", "smb", "discovery"},
		"power.":       {"reboot", "reboot_text", "reboot_going", "poweroff", "poweroff_text", "poweroff_going"},
		"err.":         {"bad_request", "unauthorized", "invalid_credentials", "not_admin", "locked", "csrf", "forbidden", "not_found", "conflict", "confirmation_invalid", "method_not_allowed", "unavailable", "update_agent_unavailable", "system", "timeout", "network"},
	}
	for p, vs := range dyn {
		for _, v := range vs {
			used[p+v] = true
		}
	}
	var missing []string
	for k := range used {
		if _, ok := ru[k]; !ok {
			missing = append(missing, k)
		}
	}
	sort.Strings(missing)
	if len(missing) > 0 {
		t.Errorf("ru.json misses %d keys: %v", len(missing), missing)
	}
	ph := regexp.MustCompile(`\{\w+\}`)
	for _, lang := range []string{"en", "uk"} {
		d := loadDict(t, lang)
		for k, v := range ru {
			tv, ok := d[k]
			if !ok {
				t.Errorf("%s.json misses %s", lang, k)
				continue
			}
			a, b := ph.FindAllString(v, -1), ph.FindAllString(tv, -1)
			sort.Strings(a)
			sort.Strings(b)
			if !slices.Equal(a, b) {
				t.Errorf("%s.json %s: placeholders %v, ru has %v", lang, k, b, a)
			}
		}
		for k := range d {
			if _, ok := ru[k]; !ok {
				t.Errorf("%s.json has %s, ru.json does not", lang, k)
			}
		}
	}
}

func TestServe(t *testing.T) {
	rec := httptest.NewRecorder()
	Index(rec, httptest.NewRequest("GET", "/", nil))
	if rec.Code != 200 || !strings.HasPrefix(rec.Header().Get("Content-Type"), "text/html") || !strings.Contains(rec.Body.String(), "/static/js/app.js") {
		t.Fatalf("index: %d %q", rec.Code, rec.Header().Get("Content-Type"))
	}
	rec = httptest.NewRecorder()
	Static(rec, httptest.NewRequest("GET", "/static/js/app.js", nil))
	if rec.Code != 200 || !strings.HasPrefix(rec.Header().Get("Content-Type"), "text/javascript") {
		t.Fatalf("app.js: %d %q", rec.Code, rec.Header().Get("Content-Type"))
	}
	etag := rec.Header().Get("ETag")
	req := httptest.NewRequest("GET", "/static/js/app.js", nil)
	req.Header.Set("If-None-Match", etag)
	rec = httptest.NewRecorder()
	Static(rec, req)
	if rec.Code != http.StatusNotModified {
		t.Fatalf("If-None-Match: %d", rec.Code)
	}
	for _, p := range []string{"/static/nope.js", "/static/index.html", "/static/../webui.go"} {
		rec = httptest.NewRecorder()
		Static(rec, httptest.NewRequest("GET", p, nil))
		if rec.Code != http.StatusNotFound {
			t.Errorf("%s: %d, want 404", p, rec.Code)
		}
	}
}
