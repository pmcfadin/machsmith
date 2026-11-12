# machsmith

This repository holds an experiment that measures LLM-written x86-64 assembly against rustc's optimizer on three CQLite kernels. The harness checks candidates differentially and measures them with hardware counters on a single pinned host.

Specs live in `docs/specs/` and implementation plans in `docs/plans/`. The experiment code is in `experiment/`.
