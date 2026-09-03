"""Independent check of the A/B statistics against scipy and brute force."""
import json, subprocess, sys, itertools, random
import numpy as np
from scipy import stats

SCR = "/private/tmp/claude-501/-Users-dao-mouse-testing/ce46051b-b80f-4438-89b7-8e9a46600d4d/scratchpad"
BIN = "/Users/dao/mouse-testing/target/debug/mouse-testing"

rng = random.Random(20240902)
cases = []

def add(id, a, b):
    cases.append({"id": id, "a": [float(x) for x in a], "b": [float(x) for x in b]})

# Continuous, no ties, small n: the exact branch on both tests.
for k in range(6):
    n = 6 + k
    a = [round(rng.gauss(10, 1.5), 6) for _ in range(n)]
    b = [round(rng.gauss(10.6, 1.5), 6) for _ in range(n)]
    add(f"cont_n{n}", a, b)

# Heavy ties: discrete actuation counts, which is the weak-input variant.
for k in range(5):
    n = 8 + 2 * k
    a = [rng.randint(0, 4) for _ in range(n)]
    b = [rng.randint(0, 4) for _ in range(n)]
    add(f"counts_n{n}", a, b)

# Zeros in the paired differences, which forces the Pratt path.
for k in range(3):
    n = 10 + 2 * k
    a = [rng.randint(3, 9) for _ in range(n)]
    b = list(a)
    for i in range(0, n, 3):
        b[i] = a[i] + rng.choice([-1, 1])
    add(f"zeros_n{n}", a, b)

# Tiny samples, where asymptotics are worst.
add("tiny_3", [1.0, 2.0, 3.0], [4.0, 5.0, 6.0])
add("tiny_4", [1.5, 2.5, 3.5, 4.5], [2.0, 3.0, 4.0, 9.0])
# Complete separation: a deep tail p-value.
add("separated", list(range(1, 13)), list(range(20, 32)))
# Larger than the exact caps, to exercise the fallbacks that used to panic.
add("big_35", [rng.gauss(20, 3) for _ in range(35)], [rng.gauss(21, 3) for _ in range(35)])
add("big_60", [rng.gauss(20, 3) for _ in range(60)], [rng.gauss(21, 3) for _ in range(60)])

json.dump(cases, open(f"{SCR}/stats_cases.json", "w"))
subprocess.run([BIN, "--stats-check", f"{SCR}/stats_cases.json", f"{SCR}/stats_out.json"],
               check=True, capture_output=True)
got = {o["id"]: o for o in json.load(open(f"{SCR}/stats_out.json"))}

def hl_unpaired_bruteforce(a, b):
    return float(np.median([x - y for x in a for y in b]))

def hl_paired_bruteforce(d):
    w = [(d[i] + d[j]) / 2 for i in range(len(d)) for j in range(i, len(d))]
    return float(np.median(w))

worst = {}
def cmp(name, mine, ref, tol=1e-9):
    if ref is None or (isinstance(ref, float) and np.isnan(ref)):
        return
    err = abs(mine - ref)
    rel = err / max(abs(ref), 1e-12)
    key = name
    prev = worst.get(key, (0.0, 0.0, ""))
    if err > prev[0]:
        worst[key] = (err, rel, cid)
    if err > tol and rel > tol:
        fails.append(f"{cid} {name}: mine={mine!r} ref={ref!r} abs={err:.3e} rel={rel:.3e}")

fails = []
for c in cases:
    cid = c["id"]
    a, b = np.array(c["a"]), np.array(c["b"])
    o = got[cid]
    d = a - b

    # Mann-Whitney U.
    r_exact = stats.mannwhitneyu(a, b, alternative="two-sided", method="exact")
    r_asym = stats.mannwhitneyu(a, b, alternative="two-sided", method="asymptotic",
                                use_continuity=True)
    cmp("mwu_u", o["mwu_u"], float(r_asym.statistic))
    # scipy's exact branch ignores ties, and ours degrades to the approximation
    # above its enumeration cap, so compare only where both are exact.
    untied = len(set(np.concatenate([a, b]))) == len(a) + len(b)
    if untied and len(a) + len(b) <= 60:
        cmp("mwu_p_exact", o["mwu_p_exact"], float(r_exact.pvalue), 1e-10)
    cmp("mwu_p_asymptotic", o["mwu_p_asymptotic"], float(r_asym.pvalue), 1e-10)

    n1, n2 = len(a), len(b)
    cmp("rank_biserial", o["mwu_rank_biserial"], 2 * o["mwu_u"] / (n1 * n2) - 1)
    cmp("prob_superiority", o["mwu_prob_superiority"], o["mwu_u"] / (n1 * n2))

    # Wilcoxon signed rank.
    if np.any(d != 0):
        nz = d[d != 0]
        if len(set(np.abs(nz))) == len(nz) and len(nz) <= 50:
            w_exact = stats.wilcoxon(d, zero_method="wilcox", method="exact")
            cmp("wsr_p_exact", o["wsr_p_exact"], float(w_exact.pvalue), 1e-10)
            cmp("wsr_statistic", o["wsr_statistic"], float(w_exact.statistic))
        w_asym = stats.wilcoxon(d, zero_method="wilcox", method="approx", correction=False)
        cmp("wsr_p_asymptotic", o["wsr_p_asymptotic"], float(w_asym.pvalue), 1e-10)

    # Hodges-Lehmann estimators against brute force.
    cmp("hl_unpaired", o["hl_unpaired"], hl_unpaired_bruteforce(a, b), 1e-9)
    cmp("hl_paired", o["hl_paired"], hl_paired_bruteforce(list(d)), 1e-9)

    # Robust summary against numpy.
    cmp("median", o["median_a"], float(np.median(a)))
    cmp("q1", o["q1_a"], float(np.percentile(a, 25)))
    cmp("q3", o["q3_a"], float(np.percentile(a, 75)))
    cmp("mad", o["mad_a"], 1.4826 * float(np.median(np.abs(a - np.median(a)))))

    # The interval must bracket its own estimate and be finite.
    for tag in ("unpaired", "paired"):
        lo, hi, est = o[f"hl_{tag}_lo"], o[f"hl_{tag}_hi"], o[f"hl_{tag}"]
        if not (np.isfinite(lo) and np.isfinite(hi)):
            fails.append(f"{cid} hl_{tag}: non-finite interval {lo}..{hi}")
        elif not (lo - 1e-9 <= est <= hi + 1e-9):
            fails.append(f"{cid} hl_{tag}: estimate {est} outside {lo}..{hi}")

    # An exhaustive permutation p-value must match a full enumeration.
    if o["perm_exhaustive"] and n1 + n2 <= 16:
        pooled = list(a) + list(b)
        obs = float(np.median(a) - np.median(b))
        cnt = tot = 0
        for idx in itertools.combinations(range(len(pooled)), n1):
            s = set(idx)
            xa = [pooled[i] for i in idx]
            xb = [pooled[i] for i in range(len(pooled)) if i not in s]
            t = float(np.median(xa) - np.median(xb))
            tot += 1
            if abs(t) >= abs(obs) - 1e-12:
                cnt += 1
        cmp("perm_p", o["perm_p"], min(1.0, cnt / tot), 1e-9)

print(f"{len(cases)} cases, {len(fails)} failure(s)")
print(f"{'statistic':22} {'max abs err':>12} {'max rel err':>12}  worst case")
for k, (e, r, cid) in sorted(worst.items()):
    print(f"{k:22} {e:12.3e} {r:12.3e}  {cid}")
for f in fails:
    print("FAIL", f)
sys.exit(1 if fails else 0)
