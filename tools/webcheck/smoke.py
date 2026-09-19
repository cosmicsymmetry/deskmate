"""smoke.py <base_url> <admin_token> <outdir> -- drive the REAL page against the REAL server in headless Chrome."""
import sys, json, time, urllib.request
from playwright.sync_api import sync_playwright
BASE, TOKEN, OUT = sys.argv[1], sys.argv[2], sys.argv[3]
R = {}
def api(method, path, body=None):
    req = urllib.request.Request(BASE + path, method=method, data=(json.dumps(body).encode() if body is not None else None), headers={"Authorization": "Bearer " + TOKEN, "Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=10) as r: return r.status, json.loads(r.read() or b"null")
st, dev = api("POST", "/v1/devices"); R["mint_device"] = {"status": st, "device_id": dev.get("device_id")}
with sync_playwright() as p:
    br = p.chromium.launch(channel="chrome", headless=True); ctx = br.new_context(viewport={"width": 1280, "height": 900}); pg = ctx.new_page()
    errs, reqs = [], []
    pg.on("pageerror", lambda e: errs.append("pageerror: " + str(e))); pg.on("console", lambda m: errs.append("console." + m.type + ": " + m.text[:200]) if m.type == "error" else None)
    pg.on("request", lambda r: reqs.append((r.method, r.url.replace(BASE, ""))))
    pg.goto(BASE + "/", wait_until="networkidle"); pg.wait_for_timeout(500)
    R["signed_out_shows_form"] = pg.get_by_role("heading", name="Sign in to Deskmate").count() == 1
    faces_before = [u for m, u in reqs if u.startswith("/v1/faces")]; events_before = [u for m, u in reqs if "/events" in u]
    pg.locator('input[type="password"]').first.fill(TOKEN); pg.get_by_role("button", name="Sign in").first.click()
    pg.wait_for_selector(".card-grid", timeout=10000); pg.wait_for_timeout(1500)
    R["signed_in_shows_window"] = pg.locator(".card-grid").count() == 1
    R["face_catalog_requests_after_signin"] = len([u for m, u in reqs if u.startswith("/v1/faces")]) - len(faces_before)
    R["event_stream_requests_after_signin"] = len([u for m, u in reqs if "/events" in u]) - len(events_before)
    pg.locator('[aria-haspopup="menu"]').first.click(); pg.wait_for_timeout(300)
    items = [t.strip() for t in pg.get_by_role("menuitem").all_inner_texts()]; R["add_menu_items"] = items
    tiles0 = pg.locator(".card-grid > li.card-tile").count()
    pg.get_by_role("menuitem", name="Pomodoro").first.click(); pg.wait_for_timeout(500)
    R["tiles_after_adding_pomodoro"] = [tiles0, pg.locator(".card-grid > li.card-tile").count()]
    save = pg.get_by_role("button", name="Save").first; R["save_enabled"] = save.is_enabled()
    save.click(); pg.wait_for_timeout(2000)
    R["save_put_status_seen"] = [m for m, u in reqs if m == "PUT" and u.endswith("/config")] != []
    img = pg.locator(".stage__screen img").first
    R["preview_png_rendered"] = img.count() == 1 and pg.evaluate("(i) => i.complete && i.naturalWidth", img.element_handle()) 
    pg.reload(wait_until="load"); pg.wait_for_selector(".card-grid", timeout=10000); pg.wait_for_timeout(1000)
    R["tiles_after_reload"] = pg.locator(".card-grid > li.card-tile").count(); R["still_signed_in_after_reload"] = pg.get_by_role("heading", name="Sign in to Deskmate").count() == 0
    st, snap = api("GET", f"/v1/app/{dev['device_id']}/snapshot") if False else (None, None)
    pg.screenshot(path=OUT + "/final.png", full_page=True); R["browser_errors"] = errs[:8]
    json.dump(R, open(OUT + "/result.json", "w"), indent=1); print(json.dumps(R, indent=1))
    # leave the page (and its SSE stream) OPEN while the server is asked to stop
    open(OUT + "/page-open", "w").write("1"); t0 = time.time()
    while not __import__("os").path.exists(OUT + "/server-stopped") and time.time() - t0 < 40: pg.wait_for_timeout(200)
    br.close()
