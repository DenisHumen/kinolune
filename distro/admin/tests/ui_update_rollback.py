# SPDX-License-Identifier: MPL-2.0
"""Browser scenario of vm-ui-e2e.sh (DIST-014): update and rollback from the admin UI.

Environment: UI_URL (https://127.0.0.1:<port>/), UI_USER, UI_PASS, UI_V1, UI_V2,
UI_OUT (screenshot folder). Needs Playwright for Python with Chromium.
"""

import os
import sys
import time

from playwright.sync_api import expect, sync_playwright

URL = os.environ["UI_URL"]
USER = os.environ["UI_USER"]
PASS = os.environ["UI_PASS"]
V1 = os.environ["UI_V1"]
V2 = os.environ["UI_V2"]
OUT = os.environ.get("UI_OUT", ".")

# A reboot plus the services coming back can take a while on a loaded build machine.
REBOOT_MS = 15 * 60 * 1000
step = 0


def log(msg):
    print(f"[ui] {time.strftime('%H:%M:%S')} {msg}", flush=True)


def shot(page, name):
    global step
    step += 1
    page.screenshot(path=os.path.join(OUT, f"{step:02d}-{name}.png"), full_page=True)


def login(page):
    expect(page.get_by_test_id("login-user")).to_be_visible(timeout=REBOOT_MS)
    page.get_by_test_id("login-user").fill(USER)
    page.get_by_test_id("login-password").fill(PASS)
    page.get_by_test_id("login-submit").click()
    expect(page.get_by_test_id("nav-updates")).to_be_visible(timeout=60_000)


def open_updates(page):
    page.get_by_test_id("nav-updates").click()
    expect(page.get_by_test_id("upd-state")).to_be_visible(timeout=60_000)


def wait_state(page, states, timeout=10 * 60 * 1000):
    sel = ", ".join(f'[data-testid="upd-state"][data-state="{s}"]' for s in states)
    page.wait_for_selector(sel, timeout=timeout)
    # No operation still running.
    expect(page.get_by_test_id("upd-op")).to_have_count(0, timeout=timeout)


def main():
    with sync_playwright() as p:
        browser = p.chromium.launch()
        ctx = browser.new_context(ignore_https_errors=True, locale="ru-RU", viewport={"width": 1366, "height": 900})
        page = ctx.new_page()
        page.on("console", lambda m: m.type == "error" and log(f"console: {m.text}"))
        page.on("pageerror", lambda e: log(f"page error: {e}"))
        page.goto(URL)

        # 1. Log in, overview.
        expect(page.get_by_test_id("login-user")).to_be_visible(timeout=60_000)
        shot(page, "login")
        page.get_by_test_id("login-user").fill(USER)
        page.get_by_test_id("login-password").fill("wrong-password")
        page.get_by_test_id("login-submit").click()
        expect(page.get_by_test_id("login-error")).to_be_visible()
        shot(page, "login-wrong-password")
        login(page)
        expect(page.get_by_test_id("cpu")).to_be_visible(timeout=60_000)
        shot(page, "overview")
        log("logged in, overview shown")

        # 2. Updates: check, V2 available, install now.
        open_updates(page)
        expect(page.get_by_test_id("upd-current")).to_have_text(V1)
        page.get_by_test_id("upd-check").click()
        wait_state(page, ["available", "downloaded", "staged"])
        expect(page.get_by_test_id("upd-available")).to_have_text(V2, timeout=60_000)
        shot(page, "update-available")
        log(f"{V2} available")
        page.get_by_test_id("upd-apply").click()
        expect(page.get_by_test_id("confirm-apply")).to_be_visible()
        shot(page, "confirm-install")
        page.get_by_test_id("confirm-apply").get_by_test_id("confirm").click()
        expect(page.locator("main h1")).to_be_visible()
        shot(page, "installing")
        log("install confirmed, waiting for the reboot")

        # 3. The page waits for the reboot and reloads to the login page by itself.
        login(page)
        open_updates(page)
        expect(page.get_by_test_id("upd-current")).to_have_text(V2, timeout=60_000)
        expect(page.get_by_test_id("upd-previous")).to_have_text(V1)
        shot(page, "updated")
        log(f"booted {V2}, previous {V1}")

        # 4. Roll back.
        page.get_by_test_id("upd-rollback").click()
        expect(page.get_by_test_id("confirm-rollback")).to_be_visible()
        shot(page, "confirm-rollback")
        page.get_by_test_id("confirm-rollback").get_by_test_id("confirm").click()
        expect(page.locator("main h1")).to_be_visible()
        shot(page, "rolling-back")
        log("rollback confirmed, waiting for the reboot")

        login(page)
        open_updates(page)
        expect(page.get_by_test_id("upd-current")).to_have_text(V1, timeout=60_000)
        expect(page.get_by_test_id("upd-previous")).to_have_text(V2)
        shot(page, "rolled-back")
        log(f"booted {V1} again, previous {V2}")

        # 5. The other pages open without errors (English this time).
        page.get_by_test_id("lang").select_option("en")
        for pid in ["dashboard", "network", "users", "sharing", "disks", "system", "advanced"]:
            nav = page.get_by_test_id(f"nav-{pid}")
            if nav.count() == 0:
                log(f"page {pid} not offered (API has no such part)")
                continue
            nav.click()
            page.wait_for_selector(f'main[data-page="{pid}"] section.panel', timeout=60_000)
            page.wait_for_timeout(500)
            if page.locator("main .alert.bad").count():
                shot(page, f"{pid}-error")
                raise AssertionError(f"{pid}: {page.locator('main .alert.bad').first.inner_text()}")
            shot(page, f"page-{pid}")
        page.get_by_test_id("logout").click()
        expect(page.get_by_test_id("login-user")).to_be_visible()
        browser.close()
    log("OK update and rollback through the UI")


if __name__ == "__main__":
    try:
        main()
    except Exception as e:  # noqa: BLE001 - report and fail
        log(f"FAILED: {e}")
        sys.exit(1)
