# vcalc — Verifier-Loop Probability Calculator

> Date: 2026-07-08
> Status: design (explore-mode artifact, NOT implemented)
> Scope: Problem A — standalone helper tool, SEPARATE from verifier-loop

## Purpose

Given verifier-loop parameters (X/Y/Z, N items, consensus config), compute:
- P(slip per round)
- Expected rounds to convergence
- Expected total delegations
- T to 95% (time units to reach 95% confidence)
- THREE cost measures: token / T / combined

Answers: "which n/m config is optimal given my constraints?"

## The Model

### Inputs

```
N    = total items under verification
Y    = P(item is initially correct)            → D₀ = N(1-Y) initial defects
X    = P(verifier catches any single defect)    → P(miss) = 1-X per defect
Z    = P(defect is fixed when round is caught)  → D_{r+1} = Binomial(D_r, 1-Z)
n/m  = consensus threshold (n of m must APPROVE)
ρ    = correlation factor (DEFERRED — see below)
```

### Round dynamics

```
Round r has D_r defects.

P(single verifier APPROVEs all items) = (1-X)^D_r
    └─ verifier must miss ALL D_r defects independently

P(slip | D_r defects, n/m consensus) = Σ_{k=n}^{m} C(m,k) · p^k · (1-p)^(m-k)
    where p = (1-X)^D_r

If CAUGHT (< n approve):
    D_{r+1} ~ Binomial(D_r, 1-Z)    ← fix phase thins defects

If SLIP (≥ n approve):
    Loop terminates with D_r surviving defects → false positive
```

### Decay trajectory

With Z=0.6, defects decay ~60% per caught round:

```
D₀ = N(1-Y)     e.g. 115×0.4 = 46
D₁ ≈ D₀×0.4     ≈ 18
D₂ ≈ D₁×0.4     ≈ 7
D₃ ≈ D₂×0.4     ≈ 3
D₄ ≈ D₃×0.4     ≈ 1
D₅ ≈ D₄×0.4     ≈ 0.4
D₆ ≈ 0
```

Slip probability INCREASES as D decreases (fewer defects = easier to miss all).

## THREE Cost Measures

The key insight: **pure token cost biases toward fewer verifiers**. Adding T (time) reveals that fewer verifiers takes MORE rounds → more wall-clock time.

### Definitions

```
Token Cost  = avgTotalDelegations              (proxy for tokens spent)
                = avgRounds × m

T Cost      = avgRounds × T                    (proxy for wall-clock time)
                each round takes T time units

Combined    = wT × T_cost + wD × token_cost   (weighted)
                currently wT = wD = 1
```

### Why T matters

```
Without T:  2/2 always wins (fewest delegations)
            → bias toward configs that are cheap but SLOW

With T:     2/2 takes 4.2T to 95% vs 3/3 takes 3.0T
            → if wall-clock matters, 3/3 may be better despite more tokens
            → eliminates the bias toward fewer verifiers
```

## Output format

```
N=115  Y=0.6  X=0.6  Z=0.6  (10k simulations each)

Consensus  P(slip   Avg     Avg Total   T to    Token     T        Combined
Config     round 1) Rounds  Delegations 95%     Cost      Cost     (wT=wD=1)
──────────────────────────────────────────────────────────────────────────────
  2/2       ~0%     ~5.2     10.4        5.5T    10.4×     5.2T     15.6
  3/3       ~0%     ~4.8     14.4        5.0T    14.4×     4.8T     19.2
  3/4       ~0%     ~4.6     18.4        4.8T    18.4×     4.6T     23.0
  4/4       ~0%     ~4.4     17.6        4.6T    17.6×     4.4T     22.0
  1/2       ~0%     ~5.8     11.6        6.2T    11.6×     5.8T     17.4
  2/3       ~0%     ~5.0     15.0        5.2T    15.0×     5.0T     20.0
  2/4       ~0%     ~5.4     21.6        5.8T    21.6×     5.4T     27.0

(Numbers above are estimates — actual values require Monte Carlo simulation)
```

Note: with D₀=46, P(slip round 1) ≈ 0 for ALL configs. Slip risk concentrates in later rounds as defects thin.

## Sensitivity analysis

The calculator should support parameter sweeps:

```
"What if X drops to 0.4 (weaker verifiers)?"
"What if Z drops to 0.3 (weak fix phase)?"
"What if N=500 (large audit)?"
"At what wT does 3/3 become cheaper than 2/2?"
```

## Implementation

- Language: Python (numpy + scipy.stats + argparse)
- Single file: `vcalc.py`
- ~80 lines core + ~40 lines reporting
- Zero framework, zero external deps beyond numpy/scipy
- CLI: `python vcalc.py --N 115 --Y 0.6 --X 0.6 --Z 0.6 --configs 2/2,3/3,3/4`

### Core algorithm (Monte Carlo)

```
for each config (n, m):
    for sim in range(10000):
        D = binomial(N, 1-Y)              # initial defects
        for round in range(maxTurn):
            p_approve = (1-X)^D           # P(verifier misses all defects)
            approvals = binomial(m, p_approve)
            if approvals >= n:
                # consensus reached
                if D == 0: legitimate_pass++
                else: slip++
                record(round, D, ...)
                break
            else:
                D = binomial(D, 1-Z)      # fix phase thins defects
```

## DEFERRED: Independence assumption trap ⚠

⚠ **DO NOT FORGET** — this model assumes i.i.d. verifiers. LLM verifiers are NOT independent:

```
Model:  P(verifier misses defect) = 1-X independently
Reality: LLMs share blind spots. Correlated failures ρ > 0.

Effective slip rate HIGHER than model predicts.

Need: correlation parameter (ρ) that inflates effective slip rate.
      Without it, numbers are dangerously optimistic.

This is deferred for now. MUST be addressed before trusting vcalc output
for production decisions.
```

**TODO**: Add `--correlation` parameter. When ρ>0, model verifiers as having shared failure modes (e.g., Beta-binomial instead of binomial).

## Key questions to resolve (before implementation)

1. How to measure X/Y/Z empirically? (Need historical data from past verifier-loop runs)
2. What constitutes "95% confidence"? (D=0 with 95% probability? Or P(slip) < 5%?)
3. Should T account for fix-phase time too? (fix ≠ verify time)
