# Rule catalogue (round 2)

One row per rule and per proposal. A rule entry is written and committed before any code: pattern,
detection, evidence, target cases, predicted ratio range. The outcome is appended after the
confirmation run. Ratios are candidate / base on the rule's target cases (below 1 is faster);
ledger ids refer to `round2/results/ledger.jsonl`. Write-up:
`docs/results/2026-11-round-2-cqlite-hotspots.md`.

- **Prediction held:** the confirmed point ratio is inside the predicted range.
- **Paid off:** the verdict is WON and no cross-check flagged a REGRESSION.

## Rules

| id | hot spot | workload | predicted ratio | outcome | prediction held | paid off | entry |
|---|---|---|---|---|---|---|---|
| `default-siphash-on-hot-map` | H1 | partition_lookup | 0.46 to 0.72 | WON, 0.4419 (0.4371 to 0.4458), T 0.0832 (R2-000031) | no (better than predicted) | yes | [entry](default-siphash-on-hot-map.md) |
| `per-cell-arc-str-clone` | H4 | read | 0.971 to 0.985 (internal route) | not attempted: predicted range above 1 − T (0.95) | — | — | [entry](per-cell-arc-str-clone.md) |
| `copy-cached-chunk-into-window` | H5 | read | 0.893 to 0.934 | TIED, 1.0132 (0.9922 to 1.0364), T 0.0881 (R2-000041) | no | no | [entry](copy-cached-chunk-into-window.md) |
| `type-name-match-per-clustering-value` | H6 | read | 0.891 to 0.937 | INCONCLUSIVE, 0.9110 (0.8909 to 0.9313), T 0.0881 (R2-000042) | yes | no | [entry](type-name-match-per-clustering-value.md) |

## Proposals (public-API changes; not made in round 2)

A proposal's measurement is a sizing run on an `r2-proposal/<rule-id>` branch, labelled "proposal,
not a fix". It is never a round 2 win, and making the change needs the owner's approval.

| id | hot spot | workload | predicted ratio | sizing result (proposal, not a fix) | prediction held | entry |
|---|---|---|---|---|---|---|
| `default-siphash-on-hot-map` | H1 | partition_lookup | 0.46 to 0.72 | 0.4459 (0.4428 to 0.4493), T 0.0832 (R2-000049); within T of the internal route (R2-000031) | no (better than predicted) | [proposal](proposals/default-siphash-on-hot-map.md) |
| `per-cell-arc-str-clone` | H4 | read | 0.87 to 0.90 | TIED, 0.9826 (0.9750 to 0.9907), T 0.05 (R2-000052) | no | [proposal](proposals/per-cell-arc-str-clone.md) |

"Paid off" does not apply to a proposal. Sizing runs that failed the noise gate: R2-000048,
R2-000050, R2-000051.
