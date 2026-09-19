"""prodcheck.py <worktree> -- a tests-only change must leave production code BYTE-IDENTICAL.
For every changed file: a pure test file may change freely; a production .rs file may change only at or after its
top-level `#[cfg(test)] mod ...` line; anything else (src .ts/.tsx/.css, Cargo.toml, docs) is reported."""
import subprocess, sys, re
wt = sys.argv[1]
TESTFILE = re.compile(r"/tests/|/tests\.rs$|\.test\.tsx?$|test_support")
MARK = re.compile(r"^#\[cfg\(test\)\][ \t]*\n(?:[ \t]*(?://[^\n]*|#\[[^\n]*\])?[ \t]*\n)*(?:pub(?:\([a-z]+\))? )?mod \w+\s*[{;]", re.M)
def prod(src):
    m = MARK.search(src); return src[:m.start()] if m else src
changed = subprocess.check_output(["git", "-C", wt, "status", "--porcelain"], text=True).splitlines()
bad = []; ok = 0
for line in changed:
    st, f = line[:2], line[3:].split(" -> ")[-1]
    if TESTFILE.search(f): ok += 1; continue
    if f.endswith("/"): ok += 1; continue
    try: before = subprocess.check_output(["git", "-C", wt, "show", f"HEAD:{f}"], text=True, stderr=subprocess.DEVNULL)
    except subprocess.CalledProcessError: before = None
    try: after = open(f"{wt}/{f}", errors="replace").read()
    except FileNotFoundError: after = None
    if f.endswith(".rs") and before is not None and after is not None and prod(before) == prod(after): ok += 1; continue
    bad.append(f"{st} {f}" + ("" if not f.endswith(".rs") else "  (production part differs)"))
print(f"prodcheck: {ok} test-only changes OK, {len(bad)} NON-TEST changes")
for b in bad: print("   NON-TEST:", b)
sys.exit(1 if bad else 0)
