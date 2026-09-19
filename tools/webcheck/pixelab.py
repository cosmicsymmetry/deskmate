"""pixelab.py <urlA> <urlB> <outdir> -- is build B pixel-identical to build A?

Serve two checkouts' mock harnesses (VITE_DESKMATE_MOCK=1 bunx vite --port N) and point this at both.
Walks every scenario x colour scheme x viewport x UI state (base, each tile's editor, the add menu,
the settings sheet) and compares full-page screenshots and visible text.

Two things make the comparison meaningful, and both were learned the hard way (see README.md):
A and B are captured in the SAME browser launch, and time is driven, not merely frozen."""
import sys, io, os, json
from playwright.sync_api import sync_playwright
from PIL import Image, ImageChops
SCENARIOS = ["default", "offline", "standalone", "unowned", "invalid", "firstrun", "empty", "carderror", "picture", "signedout"]
VIEWPORTS = {"desktop": (1280, 900), "mobile": (390, 844)}
A, B, OUT = sys.argv[1], sys.argv[2], sys.argv[3]; os.makedirs(OUT, exist_ok=True)
def walk(browser, base, vw, scheme, sc):
    ctx = browser.new_context(viewport={"width": vw[0], "height": vw[1]}, color_scheme=scheme, reduced_motion="reduce", device_scale_factor=1)
    pg = ctx.new_page(); errs = []; pg.on("pageerror", lambda e: errs.append(str(e)))
    pg.clock.install(time="2026-09-19T10:00:00Z"); pg.goto(f"{base}/?scenario={sc}", wait_until="networkidle")
    pg.clock.pause_at("2026-09-19T10:00:10Z"); pg.wait_for_timeout(250)   # deterministic: every timer due by T+10s has fired, none after
    shots = {}
    def snap(state):
        pg.mouse.move(1, 1); pg.clock.run_for(400); pg.wait_for_timeout(150)
        shots[state] = (pg.screenshot(full_page=True, animations="disabled", caret="hide"), pg.evaluate("() => document.body.innerText"))
    snap("base")
    tiles = pg.locator(".card-grid > li")
    for i in range(min(tiles.count(), 8)):
        try: tiles.nth(i).locator("button").first.click(timeout=1500); snap(f"tile{i}")
        except Exception as e: shots[f"tile{i}"] = (b"", "ERR " + type(e).__name__)
    for state, loc in (("addmenu", pg.locator('[aria-haspopup="menu"]').first), ("settings", pg.get_by_role("button", name="ettings").first)):
        try:
            if loc.count(): loc.click(timeout=1500); snap(state); pg.keyboard.press("Escape")
        except Exception as e: shots[state] = (b"", "ERR " + type(e).__name__)
    ctx.close(); return shots, errs
res = {"identical": 0, "aa_noise": [], "different": [], "text_different": [], "states_mismatch": [], "pageerrors": []}
with sync_playwright() as p:
    br = p.chromium.launch(channel="chrome", headless=True)
    for vn, vw in VIEWPORTS.items():
        for scheme in ("light", "dark"):
            for sc in SCENARIOS:
                a, ea = walk(br, A, vw, scheme, sc); b, eb = walk(br, B, vw, scheme, sc); tag = f"{vn}/{scheme}/{sc}"
                if ea or eb: res["pageerrors"].append([tag, ea, eb])
                if set(a) != set(b): res["states_mismatch"].append([tag, sorted(a), sorted(b)])
                for st in sorted(set(a) & set(b)):
                    (pa, ta), (pb, tb) = a[st], b[st]
                    if ta != tb: res["text_different"].append(f"{tag}/{st}")
                    if pa == pb: res["identical"] += 1; continue
                    ia, ib = Image.open(io.BytesIO(pa)).convert("RGB"), Image.open(io.BytesIO(pb)).convert("RGB")
                    if ia.size == ib.size:
                        d = ImageChops.difference(ia, ib); bb = d.getbbox()
                        if not bb: res["identical"] += 1; continue
                        vals = [max(px) for px in d.crop(bb).getdata() if max(px)]; info = f"{len(vals)}px maxdelta={max(vals)} bbox={bb}"
                    else: info = f"size {ia.size} vs {ib.size}"
                    if ia.size == ib.size and ta == tb and max(vals) <= 16 and len(vals) <= 400: res["aa_noise"].append(f"{tag}/{st}: {info}"); continue
                    res["different"].append(f"{tag}/{st}: {info}"); fn = f"{OUT}/{tag}/{st}".replace("/", "__"); ia.save(fn + ".A.png"); ib.save(fn + ".B.png")
    br.close()
json.dump(res, open(f"{OUT}/result.json", "w"), indent=1)
print(f"IDENTICAL={res['identical']} AA_NOISE(<=16 levels,<=400px,same text)={len(res['aa_noise'])} DIFFERENT={len(res['different'])} TEXT_DIFFERENT={len(res['text_different'])} STATE_MISMATCH={len(res['states_mismatch'])} PAGEERRORS={len(res['pageerrors'])}")
for x in res["different"][:25]: print("   PIXELS", x)
for x in res["text_different"][:10]: print("   TEXT", x)
for x in res["states_mismatch"][:5]: print("   STATES", x)
for x in res["pageerrors"][:5]: print("   PAGEERROR", x)
