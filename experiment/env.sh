# Sourced by every script in the experiment. Do not execute.
unset CARGO_TARGET_DIR
export MSX_ROOT=/home/dev/projects/machsmith
export MSX_WORK=$MSX_ROOT/work
export MSX_LLVM_BIN="$(rustc --print sysroot)/lib/rustlib/x86_64-unknown-linux-gnu/bin"

# msx_cargo <target-name> <cargo args...>
msx_cargo() {
    local name=$1
    shift
    CARGO_TARGET_DIR="$MSX_WORK/target/$name" cargo "$@"
}

# Returns 1 with a message when the environment is unsafe or incomplete.
# Callers: msx_require_env || exit 1
msx_require_env() {
    if [ -n "${CARGO_TARGET_DIR:-}" ]; then
        case "$CARGO_TARGET_DIR" in
            "$MSX_WORK"/*) ;;
            *) echo "msx: CARGO_TARGET_DIR=$CARGO_TARGET_DIR is outside $MSX_WORK/" >&2; return 1 ;;
        esac
    fi
    local t
    for t in gcc perf python3; do
        command -v "$t" >/dev/null 2>&1 || { echo "msx: missing tool: $t" >&2; return 1; }
    done
    for t in llvm-profdata llvm-objdump llvm-nm llvm-readobj; do
        [ -x "$MSX_LLVM_BIN/$t" ] || { echo "msx: missing $MSX_LLVM_BIN/$t" >&2; return 1; }
    done
    return 0
}
