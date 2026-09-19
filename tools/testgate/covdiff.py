"""covdiff.py <before.lcov> <after.lcov> [worktree] -- production lines covered BEFORE that are NOT covered AFTER.
Only meaningful when production files are unchanged between the two runs (a tests-only change).
A production .rs file's INLINE `#[cfg(test)] mod` is test code: with [worktree] given, lines at or after that marker
are ignored (above it the file is byte-identical by construction, so production line numbers are stable)."""
import sys, re, collections, os
TEST = re.compile(r"/tests/|/examples/|/tests\.rs$|test_support|app_api/contract\.rs$|faces/(cases|golden)\.rs$|lvgl-sim/src/cases|/\.cargo/|/rustc/|/target/")
ROOT = sys.argv[3] if len(sys.argv) > 3 else None
MARK = re.compile(r"^#\[cfg\(test\)\][ \t]*\n(?:[ \t]*(?://[^\n]*|#\[[^\n]*\])?[ \t]*\n)*(?:pub(?:\([a-z]+\))? )?mod \w+\s*[{;]", re.M)
_cut = {}
def cutoff(f):
    """first line of the inline test module in companion/<f>, or a huge number"""
    if f not in _cut:
        n = 10**9
        if ROOT and f.endswith(".rs"):
            try:
                src = open(os.path.join(ROOT, "companion", f), errors="replace").read(); m = MARK.search(src)
                if m: n = src[:m.start()].count("\n") + 1
            except OSError: pass
        _cut[f] = n
    return _cut[f]
def load(p):
    cov = collections.defaultdict(set); tot = collections.defaultdict(set); f = None
    for line in open(p):
        if line.startswith("SF:"): f = line[3:].strip(); f = f.split("/companion/")[-1] if "/companion/" in f else f
        elif line.startswith("DA:") and f and not TEST.search(f):
            n, c = line[3:].strip().split(",")[:2]
            if int(n) >= cutoff(f): continue
            tot[f].add(int(n))
            if int(c) > 0: cov[f].add(int(n))
    return cov, tot
b, bt = load(sys.argv[1]); a, at = load(sys.argv[2])
lost = {f: sorted(b[f] - a.get(f, set())) for f in b if b[f] - a.get(f, set())}
gained = sum(len(a[f] - b.get(f, set())) for f in a)
cb, ca = sum(len(v) for v in b.values()), sum(len(v) for v in a.values()); tb = sum(len(v) for v in bt.values())
print(f"production lines instrumented: {tb}  covered before: {cb} ({cb/tb*100:.1f}%)  after: {ca}  newly covered: {gained}  LOST: {sum(len(v) for v in lost.values())}")
for f, ls in sorted(lost.items(), key=lambda x: -len(x[1]))[:25]: print(f"   LOST {len(ls):4d}  {f}: {ls[:12]}{' ...' if len(ls) > 12 else ''}")
sys.exit(1 if lost else 0)
