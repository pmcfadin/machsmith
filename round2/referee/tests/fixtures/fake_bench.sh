#!/usr/bin/env bash
# A stand-in for a CQLite Criterion bench binary (tests only). Ignores its arguments.
# FAKE_FACTOR        per-iteration time multiplier (default 1.0)
# FAKE_FACTOR_CASES  comma list; when set, FAKE_FACTOR applies only to these cases
# FAKE_CASES         comma list of Criterion full_ids "group/function" (default g/a,g/b)
# FAKE_EXIT          exit code (default 0)
# FAKE_SLEEP         seconds to sleep before writing (default 0.2)
# Writes <CRITERION_HOME>/<group>/<function>/new/{benchmark,sample}.json in Criterion 0.5.1 format,
# 50 samples, time per iteration = 1000 ns x factor x (1 + jitter), jitter uniform in +-0.5%.
# The jitter is seeded by the case's position only, so two arms with the same factor report
# identical medians (an A/A ratio is exactly 1.0) and tests stay deterministic.
#
# The work is one perl process (exec, no forks): a real bench is a single process, and forking
# helpers on a pinned core would show up as involuntary context switches and trip the noise gate.
exec perl -e '
use strict; use warnings;
use File::Path qw(make_path);
my $home = $ENV{CRITERION_HOME} // die "CRITERION_HOME unset\n";
my $factor = $ENV{FAKE_FACTOR} // 1.0;
my %only = map { $_ => 1 } grep { length } split /,/, ($ENV{FAKE_FACTOR_CASES} // "");
my @cases = split /,/, ($ENV{FAKE_CASES} // "g/a,g/b");
select(undef, undef, undef, $ENV{FAKE_SLEEP} // 0.2);
my $n = 0;
for my $c (@cases) {
    srand(7919 + $n++);
    my ($g, $f) = split m{/}, $c, 2;
    my $k = (!%only || $only{$c}) ? $factor : 1.0;
    my $dir = "$home/$g/$f/new";
    make_path($dir);
    open my $b, ">", "$dir/benchmark.json" or die "$dir: $!\n";
    print $b qq({"group_id":"$g","function_id":"$f","value_str":null,"throughput":[],"full_id":"$c","directory_name":"$g/$f","title":"$c"}\n);
    close $b;
    my @iters = map { $_ * 10 } 1 .. 50;
    my @times = map { sprintf "%.4f", $_ * 1000 * $k * (1 + (rand() - 0.5) * 0.01) } @iters;
    open my $s, ">", "$dir/sample.json" or die "$dir: $!\n";
    print $s q({"sampling_mode":"Linear","iters":[), join(",", @iters), q(],"times":[), join(",", @times), "]}\n";
    close $s;
}
my $code = $ENV{FAKE_EXIT} // 0;
print STDERR "fake bench: failing with exit $code\n" if $code;
exit $code;
' -- "$@"
