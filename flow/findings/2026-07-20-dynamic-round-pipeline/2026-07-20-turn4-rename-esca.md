# Explore Turn 4 — rename Esc → esca

> Date: 2026-07-20
> Phase: explore (ospx:explore)
> Status: user locked LD12 (Esc→esca rename); assistant applied rename across all prior concepts

## [user] (verbatim — trust chain L1)

> qChange the Esc to esca (to avoid mistaken with the Esc in the keyboard);

## [assistant]

Renamed: **Esc → esca**, **EscT → escaT**.

| Concept | Abbr | Notes |
|---------|------|-------|
| Dump verifier | **D** | adapter ref |
| Smart verifier | **S** | adapter ref |
| Phase | **P** | `{ dump, smart, threshold }` |
| Pipeline | **PL** | `Vec<Phase>` |
| Escalation counter | **esca** | consecutive Gate-pass/Confirm-reject |
| Escalation threshold | **escaT** | config; `esca ≥ escaT` → switch to PL-E |

Counter lifecycle unchanged:
```
Gate pass + Confirm reject  →  esca++
Confirm pass                →  esca = 0
Mixed pass + Final reject   →  esca = 0
Mixed reject                →  esca = 0
```

Outputs unchanged (rename only):
```
PL-D:  "2+1/2"  (Gate 2D + Confirm 1S)
PL-E:  "1+2+1/2"  (Mixed 1D+2S + Final 1S), m=3
```

Config schema unchanged, just field renamed in my prose:
```json
{
  "n": 2, "m": 2,
  "dumpAdapter": "pi",
  "smartAdapter": "hermes",
  "confirmCount": 1,
  "escaT": 2
}
```

Ready to capture as OpenSpec proposal, or keep refining?

## Status at end of turn 4

Rename applied. Final concept set stable: D, S, P (Phase), PL (Pipeline), esca, escaT. 12 locked decisions (LD1–LD12), 8 open threads remain. User then invoked `findings` persist command (separate, not part of explore).
