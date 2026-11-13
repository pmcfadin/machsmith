#!/usr/bin/env bash
# Dry check of test.sh's decide(): sample summary lines -> pass | fallback | fail.
set -u
TESTSH_SOURCE_ONLY=1 . /home/dev/projects/machsmith/round2/test.sh
rc=0
check() { # name expected result cs ti
    local got; got=$(decide "$3" "${4:-}" "${5:-tree-integrity: PASS}")
    if [ "$got" = "$2" ]; then echo "ok   $1 -> $got"; else echo "FAIL $1: expected $2, got $got"; rc=1; fi
}
BEHIND='component-set: FAIL-CLOSED (#3544) — this tree is BEHIND origin/main abc; 2 of its 36 component(s) MISSING from this gate script: x,y; a PASS here would certify'
INDET='component-set: FAIL-CLOSED (#3544) — 2 component(s) missing vs origin/main abc (x) and the ancestry probe could not tell BEHIND from a deliberate removal (shallow); overall verdict FAIL'
UNCOMM='component-set: FAIL-CLOSED (#3544) — 1 of the 36 component(s) missing vs origin/main abc are PRESENT in the committed component set AT HEAD (x), so the removal is an UNCOMMITTED WORKING-TREE EDIT'
UNMEAS='component-set: FAIL-CLOSED (#3544) — baseline NOT measured (fetch-failed: x); no PASS may be derived'
OBJ='component-set: FAIL-CLOSED (#3544) — object-store-corrupt: cannot read objects'
NOTCANON='component-set: FAIL-CLOSED (#3544) — baseline NOT measured (remote-not-canonical: origin does not name the canonical upstream github.com/pmcfadin/cqlite — rejected on: local. THE URL IS DELIBERATELY NOT RENDERED (#3544 / job 282): five successive findings leaked a credential out of this one value (raw, then unflattened, then unredacted stderr, then scp-form, then query strings and multi-@ authorities), and the set of places a secret can hide in a URL does not close — so the axis is named instead of the bytes. A fork, a re-pointed remote, an unauthenticated transport (http/git), a non-default port or a credential-bearing URL is a DIFFERENT baseline, and no PASS may be derived from one); no PASS may be derived from an unmeasured baseline; overall verdict FAIL'
check remote-not-canonical fallback FAIL "$NOTCANON"
check behind-origin-main fallback FAIL "$BEHIND"
check ancestry-indeterminate fallback FAIL "$INDET"
check uncommitted-edit fail FAIL "$UNCOMM"
check baseline-unmeasured fail FAIL "$UNMEAS"
check object-store-corrupt fail FAIL "$OBJ"
check tree-mutated-midrun fail FAIL 'component-set: PASS (36/36 vs origin/main abc)' 'tree-integrity: FAIL (tree-mutated-midrun; changed: x)'
check tree-capture-failed-with-behind fail FAIL "$BEHIND" 'tree-integrity: FAIL (tree-capture-failed)'
check plain-test-failure fail FAIL 'component-set: PASS (36/36 vs origin/main abc)'
check pass pass PASS 'component-set: PASS (36/36 vs origin/main abc)'
check error fail ERROR "$BEHIND"
check refused fail REFUSED "$BEHIND"
check unknown fail WEIRD ""
check partial fail PARTIAL ""
check pass-but-tree-fail fail PASS "" 'tree-integrity: FAIL (x)'
exit $rc
