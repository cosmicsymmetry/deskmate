#!/usr/bin/env python3
"""ROD-4 revision 6: per-verb and retained-selector measurement on the VM.

Usage: sudo python3 measure6.py faces.tar hn-front-page.captured.json

Mirrors Vault's 2026-10-01 method (reproduction bundle 703ba01d): extract the
faces export to a temporary directory, copy the INSTALLED node_modules, replace
only the two HTTP adapter exports with offline fixtures, and drive the real
main.ts CLI in fresh processes inside a hardened transient unit as nobody with
private networking. Adds: the fifteen-face catalog, Hacker News (render, views,
every staged page, tap), ink-landscape (an offline plugin that declares a tap),
and the retained tap-worker (startup, per-exchange wall and CPU, RSS).
Nothing is installed into production and production is not restarted.
"""
import hashlib
import json
import os
import pathlib
import subprocess
import sys
import tarfile
import tempfile

FACES_TAR = sys.argv[1]
HN_JSON = pathlib.Path(sys.argv[2]).read_text()

FIXTURE = r'''
// --- ROD-4 measurement fixtures: offline replacements for the two adapter exports ---
const githubFixture = { public_repos: 123, followers: 4567, following: 89 };
const hnStories: Array<{ id: number }> = __HN_JSON__;
export const request: RequestFn = async () => ({
  status: 200,
  body: JSON.stringify(githubFixture),
  json: githubFixture,
});
export const fetchText: FetchText = async (url) => {
  if (url.includes("geocoding")) {
    return JSON.stringify({ results: [{ latitude: 41.7, longitude: 44.8, name: "Fixture", country: "GE" }] });
  }
  if (url.includes("topstories")) {
    return JSON.stringify(hnStories.map((s) => s.id));
  }
  const item = url.match(/item\/(\d+)\.json/);
  if (item) {
    const story = hnStories.find((s) => s.id === Number(item[1]));
    if (story === undefined) throw new Error(`fixture has no item ${item[1]}`);
    return JSON.stringify(story);
  }
  return JSON.stringify({
    current: { temperature_2m: 22, weather_code: 0, is_day: 1, time: "2026-10-04T12:00" },
    daily: { time: ["2026-10-04", "2026-10-05"], temperature_2m_max: [24, 25], temperature_2m_min: [15, 16], weather_code: [0, 1] },
    hourly: { time: ["2026-10-04T12:00", "2026-10-04T13:00"], temperature_2m: [22, 23], weather_code: [0, 1], is_day: [1, 1] },
  });
};
'''

DRIVER = r'''
import base64, json, os, struct, subprocess, sys, time
faces = os.path.join(os.path.dirname(__file__), "companion/faces")
# The server passes its denylist path in this variable (faces_package.rs builder);
# an EMPTY policy here means every folder is probed, which is the cost being measured.
# The live file under /var/lib/deskmate is unreadable to nobody, and unreadable means
# "withdraw everything", which is correct in production and useless for a measurement.
DENYLIST = os.path.join(os.path.dirname(__file__), "empty-denylist.json")
ENV = ["/usr/bin/env", "-i", "PATH=/usr/local/bin:/usr/bin:/bin",
       "BUN_RUNTIME_TRANSPILER_CACHE_PATH=0", "NO_COLOR=1",
       "DESKMATE_PLUGIN_DENYLIST=" + DENYLIST]
BUN = "/usr/local/bin/bun"
MAIN = faces + "/src/main.ts"

def emit(**kw):
    print(json.dumps(kw), flush=True)

def metrics(stderr):
    for line in stderr.splitlines():
        if line.startswith("METRICS "):
            u, s, m, e = line.split()[1:]
            return {"cpu": round(float(u) + float(s), 3), "rss_kib": int(m), "wall": float(e)}
    return None

def run_verb(label, kind, settings, verb, state=None, view=None, expect_exit=0, n=3):
    out = []
    for i in range(n):
        body = {"kind": kind, "settings": settings, "event": {"taps": 1, "point": None}}
        if state is not None:
            body["state"] = state
        if view is not None:
            body["view"] = view
        p = subprocess.run(["/usr/bin/time", "-f", "METRICS %U %S %M %e", *ENV, BUN, MAIN, verb],
                           input=json.dumps(body), text=True, capture_output=True, timeout=60)
        m = metrics(p.stderr)
        result = None
        valid = p.returncode == expect_exit
        if p.returncode == 0:
            try:
                result = json.loads(p.stdout)
            except Exception as error:
                valid = False
                result = {"parse_error": str(error)}
            if verb == "render" and isinstance(result, dict) and "png" in result:
                png = base64.b64decode(result["png"])
                valid = valid and struct.unpack(">II", png[16:24]) == (448, 368)
                result = {"png_bytes": len(png), "state": result.get("state")}
            if verb == "describe" and isinstance(result, list):
                result = {"catalog_count": len(result), "kinds": [f.get("kind") for f in result]}
        err = p.stderr.replace("\n", " | ")[:300]
        emit(m="M1" if label != "hn" else "M3", label=label, kind=kind, verb=verb, view=view,
             n=i + 1, exit=p.returncode, valid=valid, **(m or {}), result=(None if verb == "render" and i else result), stderr=err)
        out.append((result, valid))
        if not valid:
            raise SystemExit(f"unexpected outcome for {label} {verb}: exit {p.returncode}: {err}")
    return out[0][0]

# ---- M1: fresh children on the fifteen-face catalog ----
run_verb("catalog", "weather", {}, "describe")
run_verb("weather", "weather", {"location": "Fixture"}, "render")
run_verb("weather", "weather", {"location": "Fixture"}, "views")
run_verb("github", "github-stats", {"user": "fixture"}, "render")
run_verb("github", "github-stats", {"user": "fixture"}, "views")
INK = {"palette": "paper", "change": "daily"}
run_verb("ink", "ink-landscape", INK, "render")
run_verb("ink", "ink-landscape", INK, "views")
run_verb("ink", "ink-landscape", INK, "tap", expect_exit=1)

# ---- M3: a paging built-in end to end ----
hn_first = run_verb("hn", "hackernews", {}, "render")
hn_state = hn_first["state"]
hn_views = run_verb("hn", "hackernews", {}, "views", state=hn_state)["views"]
emit(m="M3", note="hackernews views", views=hn_views)
staged = [v for v in hn_views if v][:3]
for v in staged:
    run_verb("hn", "hackernews", {}, "render", state=hn_state, view=v)
hn_tap = run_verb("hn", "hackernews", {}, "tap", state=hn_state)

# ---- M2: the retained tap-worker ----
def proc_stats(pid):
    with open(f"/proc/{pid}/status") as f:
        rss = next(int(l.split()[1]) for l in f if l.startswith("VmRSS:"))
    with open(f"/proc/{pid}/stat") as f:
        fields = f.read().rsplit(")", 1)[1].split()
    ticks = os.sysconf("SC_CLK_TCK")
    return {"rss_kib": rss, "cpu": round((int(fields[11]) + int(fields[12])) / ticks, 3)}

t0 = time.monotonic()
w = subprocess.Popen([*ENV, BUN, MAIN, "tap-worker"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                     stderr=subprocess.DEVNULL, text=True, bufsize=1)

def exchange(kind, settings, state):
    body = json.dumps({"kind": kind, "settings": settings, "state": state, "event": {"taps": 1, "point": None}})
    t = time.monotonic()
    w.stdin.write(body + "\n"); w.stdin.flush()
    line = w.stdout.readline()
    dt = time.monotonic() - t
    answer = json.loads(line)
    return dt, answer

dt, a = exchange("hackernews", {}, hn_state)
emit(m="M2", phase="startup+first", wall=round(time.monotonic() - t0, 4), first_exchange_wall=round(dt, 4),
     code=a.get("code"), retire=a.get("retire"), view=(a.get("result") or {}).get("view"), **proc_stats(w.pid))
for phase, kind, settings, state, count, expect in [
    ("hn-builtin", "hackernews", {}, hn_state, 10, 0),
    ("ink-plugin", "ink-landscape", INK, None, 10, 1),
    ("hn-after-plugin", "hackernews", {}, hn_state, 5, 0),
]:
    before = proc_stats(w.pid)
    walls = []
    for _ in range(count):
        dt, a = exchange(kind, settings, state)
        walls.append(round(dt, 4))
        if a.get("code") != expect:
            emit(m="M2", phase=phase, unexpected=a)
            raise SystemExit(f"unexpected selector answer in {phase}: {a}")
        if a.get("retire"):
            emit(m="M2", phase=phase, note="worker asked to retire", answer=a)
            break
    after = proc_stats(w.pid)
    walls_sorted = sorted(walls)
    emit(m="M2", phase=phase, count=len(walls), walls=walls,
         wall_median=walls_sorted[len(walls) // 2], wall_max=walls_sorted[-1],
         cpu_per_exchange=round((after["cpu"] - before["cpu"]) / max(len(walls), 1), 4),
         rss_before_kib=before["rss_kib"], rss_after_kib=after["rss_kib"],
         error=(a.get("error") or "")[:120])
final = proc_stats(w.pid)
emit(m="M2", phase="final", **final)
w.stdin.close(); w.wait(timeout=10)
emit(m="M2", phase="exit", code=w.returncode)
'''

with tempfile.TemporaryDirectory(prefix="rod4-rev6-") as root:
    os.chmod(root, 0o755)
    with tarfile.open(FACES_TAR) as archive:
        archive.extractall(root, filter="data")
    faces = pathlib.Path(root) / "companion/faces"
    subprocess.run(["cp", "-a", "/var/lib/private/deskmate/faces/node_modules", str(faces / "node_modules")], check=True)
    http = faces / "src/kit/http.ts"
    original = http.read_text()
    needles = [
        "export const request: RequestFn = createRequest();",
        "export const fetchText: FetchText = createFetchText();",
    ]
    for needle in needles:
        if needle not in original:
            raise SystemExit(f"http.ts no longer has the expected export line: {needle}")
    patched = original
    for needle in needles:
        patched = patched.replace(needle, "")
    patched += FIXTURE.replace("__HN_JSON__", HN_JSON)
    http.write_text(patched)
    driver = pathlib.Path(root) / "driver.py"
    driver.write_text(DRIVER)
    (pathlib.Path(root) / "empty-denylist.json").write_text("[]\n")
    print(json.dumps({
        "archive_sha256": hashlib.sha256(open(FACES_TAR, "rb").read()).hexdigest(),
        "main_sha256": hashlib.sha256((faces / "src/main.ts").read_bytes()).hexdigest(),
        "original_http_sha256": hashlib.sha256(original.encode()).hexdigest(),
        "patched_http_sha256": hashlib.sha256(http.read_bytes()).hexdigest(),
        "bun": subprocess.check_output(["/usr/local/bin/bun", "--version"], text=True).strip(),
        "plugins": sorted(p.name for p in (faces / "plugins").iterdir() if p.is_dir()),
    }), flush=True)
    subprocess.run(["chmod", "-R", "a+rX", root], check=True)
    subprocess.run([
        "systemd-run", "--quiet", "--wait", "--pipe", "--collect", f"--unit=rod4-rev6-{os.getpid()}",
        "-p", "User=nobody", "-p", "PrivateNetwork=yes", "-p", "NoNewPrivileges=yes",
        "-p", "ProtectSystem=strict", "-p", "ProtectHome=yes", "-p", "MemoryMax=768M",
        "-p", "CPUQuota=50%", "-p", "RuntimeMaxSec=600",
        "/usr/bin/python3", str(driver),
    ], check=True)
    print("CLEANUP temporary source, dependencies and unit removed", flush=True)
