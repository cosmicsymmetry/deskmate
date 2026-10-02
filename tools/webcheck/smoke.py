"""smoke.py <base_url> <admin_token> <outdir> -- drive the REAL page against the REAL server in headless Chrome."""
import json
from pathlib import Path
import re
import sys
import time
import urllib.request
from playwright.sync_api import expect, sync_playwright

BASE, TOKEN, OUT = sys.argv[1], sys.argv[2], Path(sys.argv[3])
R = {}


def api(method, path, body=None):
    req = urllib.request.Request(BASE + path, method=method, data=(json.dumps(body).encode() if body is not None else None), headers={"Authorization": "Bearer " + TOKEN, "Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=10) as response:
        return response.status, json.loads(response.read() or b"null")


with sync_playwright() as p:
    br = p.chromium.launch(channel="chrome", headless=True)
    ctx = br.new_context(viewport={"width": 1280, "height": 900})
    # Observe the app's real EventSource, including readyState before SIGTERM.
    ctx.add_init_script("""window.__smokeStreams = [];
        const NativeEventSource = window.EventSource;
        window.EventSource = class extends NativeEventSource {
            constructor(...args) { super(...args); window.__smokeStreams.push(this); }
        };""")
    pg = ctx.new_page()
    errs, reqs = [], []
    pg.on("pageerror", lambda e: errs.append("pageerror: " + str(e)))
    pg.on("console", lambda m: errs.append("console.error: " + m.text[:200] + " " + str(m.location)) if m.type == "error" else None)
    pg.on("request", lambda r: reqs.append((r.method, r.url.replace(BASE, ""))))
    try:
        pg.goto(BASE + "/", wait_until="load")
        expect(pg.get_by_role("heading", name="Set up this server", exact=True)).to_be_visible()
        R["setup_shows_form"] = True
        code = re.search(r"Deskmate setup code: ([A-Z0-9-]+)", (OUT / "server.log").read_text())
        assert code, "server did not log a setup code"
        pg.get_by_label("Setup code").fill(code[1])
        pg.get_by_label("Email", exact=True).fill("smoke@example.com")
        pg.get_by_role("button", name="Set up", exact=True).click()
        expect(pg.get_by_role("heading", name="Add your panel", exact=True)).to_be_visible()
        R["setup_reaches_add_panel"] = True
        # The admin API assigns this identity to the newly created owner; no USB provisioning.
        status, dev = api("POST", "/v1/devices")
        assert status == 200 and dev.get("device_id"), "device mint failed"
        R["mint_device"] = {"status": status, "device_id": dev["device_id"]}
        pg.reload(wait_until="load")
        expect(pg.locator(".card-grid")).to_be_visible()
        R["signed_in_shows_window"] = True
        pg.wait_for_function("window.__smokeStreams.some(s => s.readyState === EventSource.OPEN)")
        pg.wait_for_function("performance.getEntriesByType('resource').some(r => new URL(r.name).pathname === '/v1/faces')")
        expect(pg.locator(".save-bar:visible").get_by_text("Everything is up to date", exact=True)).to_be_visible()
        R["face_catalog_requests_after_setup"] = len([u for _, u in reqs if u.startswith("/v1/faces")])
        assert R["face_catalog_requests_after_setup"] > 0
        pg.locator('[aria-haspopup="menu"]').first.click()
        R["add_menu_items"] = [t.strip() for t in pg.get_by_role("menuitem").all_inner_texts()]
        tiles = pg.locator(".card-grid > li.card-tile:not(.card-tile--add)")
        tiles0 = tiles.count()
        with pg.expect_response(lambda r: r.url.endswith("/config/validate") and
                                len(json.loads(r.request.post_data_json["json"])["cards"]) == tiles0 + 1) as validated:
            pg.get_by_role("menuitem", name=re.compile(r"^Pomodoro")).click()
        assert validated.value.ok, "new card validation failed"
        expect(tiles).to_have_count(tiles0 + 1)
        R["tiles_after_adding_pomodoro"] = [tiles0, tiles.count()]
        save = pg.get_by_role("button", name="Save to server", exact=True)
        expect(save).to_be_enabled()
        with pg.expect_response(lambda r: r.request.method == "PUT" and r.url.endswith("/config")) as saved:
            save.click()
        assert saved.value.ok, f"save failed: HTTP {saved.value.status}"
        R["save_put_status"] = saved.value.status
        submitted = json.loads(saved.value.request.post_data_json["json"])
        assert len(submitted["cards"]) == tiles0 + 1, "save omitted the new card"
        expect(save).to_be_disabled()
        img = pg.locator(".stage__screen img").first
        expect(img).to_be_visible()
        dimensions = img.evaluate("async i => { await i.decode(); return [i.naturalWidth, i.naturalHeight]; }")
        assert dimensions == [448, 368], f"preview did not decode: {dimensions}"
        R["preview_png_dimensions"] = dimensions
        pg.reload(wait_until="load")
        expect(pg.locator(".card-grid")).to_be_visible()
        expect(tiles).to_have_count(tiles0 + 1)
        account = ctx.request.get(BASE + "/v1/app/account")
        assert account.ok, f"session did not survive reload: HTTP {account.status}"
        snapshot = ctx.request.get(BASE + f"/v1/app/{dev['device_id']}/snapshot")
        assert snapshot.ok and len(snapshot.json()["config"]["cards"]) == tiles0 + 1, "server lost the saved card"
        R["tiles_after_reload"] = tiles.count()
        R["session_after_reload"] = account.status
        pg.wait_for_function("window.__smokeStreams.some(s => s.readyState === EventSource.OPEN)")
        R["event_stream_open_before_shutdown"] = True
        assert not [e for e in errs if e.startswith("pageerror:")], f"browser exceptions: {errs}"
        pg.screenshot(path=str(OUT / "final.png"), full_page=True)
        (OUT / "page-open").write_text("1")
        deadline = time.monotonic() + 40
        while not (OUT / "server-stopped").exists() and time.monotonic() < deadline:
            pg.wait_for_timeout(200)
        assert (OUT / "server-stopped").exists(), "server shutdown was not reported"
    finally:
        R["browser_errors"] = errs[:8]
        (OUT / "result.json").write_text(json.dumps(R, indent=1))
        print(json.dumps(R, indent=1))
        br.close()
