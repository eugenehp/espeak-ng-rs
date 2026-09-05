#!/usr/bin/env bash
#
# publish.sh - publish the espeak-ng-rs workspace to crates.io.
#
# The workspace is ~126 crates, which is far more than crates.io's rate
# limiter allows in one burst, so this script paces itself:
#
#   new crate           burst 10, then 1 per 10 minutes
#   new version         burst 30, then 1 per 1 minute
#
# Pacing is done with a GCRA token bucket (see throttle()), which spends the
# burst first and then settles into the sustained rate. Time already spent
# building and uploading counts against the interval, so verification is
# mostly free. If crates.io returns 429 anyway - because an earlier run
# already drained the bucket - the retry loop parses the "try again after"
# timestamp out of the error and sleeps until exactly then.
#
# Every step is resumable: a crate whose version is already on the sparse
# index is skipped, so re-running after a failure costs nothing.
#
#   ./scripts/publish.sh                 # plan: preflight + order + ETA
#   ./scripts/publish.sh package         # offline `cargo package` of each crate
#   ./scripts/publish.sh publish         # the real thing
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# ---------------------------------------------------------------- defaults --

MODE=plan

# crates.io rate limiter defaults. Override if crates.io changes them or if
# you have been granted a higher limit.
NEW_BURST=${NEW_BURST:-10}
NEW_RATE=${NEW_RATE:-600}
EXISTING_BURST=${EXISTING_BURST:-30}
EXISTING_RATE=${EXISTING_RATE:-60}

# Assume the buckets start full. Pass --assume-drained after a run that was
# interrupted part-way, to pace from empty instead of eating a 429 first.
ASSUME_FULL=1

MAX_WAIT=${MAX_WAIT:-5400}      # refuse to sleep longer than this on a 429
MAX_RETRIES=${MAX_RETRIES:-6}
INDEX_TIMEOUT=${INDEX_TIMEOUT:-300}
EST_PUBLISH_SECS=25             # rough per-crate build+upload, for the ETA only

ONLY=""
EXCLUDE=""
FROM=""
LIMIT=0
ASSUME_YES=0
ALLOW_DIRTY=0
NO_VERIFY=0
SKIP_DATA_CHECK=0
SKIP_TESTS=1
NO_MAIN=0

STATE_FILE="${STATE_FILE:-$ROOT/target/publish-state.tsv}"
LOG_FILE=""

usage() {
    # The header comment block, minus the shebang, is the overview.
    awk 'NR > 1 { if ($0 !~ /^#/) exit; sub(/^# ?/, ""); print }' "${BASH_SOURCE[0]}"
    cat <<'USAGE'

Modes:
  plan               preflight checks, publish order, ETA (default; no writes)
  package            `cargo package` every crate offline, verifying it builds
  publish            publish for real, paced against the rate limits

Selection:
  --only REGEX       only crates whose name matches
  --exclude REGEX    skip crates whose name matches
  --from NAME        start at NAME in the publish order (resume by hand)
  --limit N          stop after N publishes this run
  --no-main          do not publish the espeak-ng crate itself

Behaviour:
  -y, --yes          do not prompt for confirmation
  --allow-dirty      pass --allow-dirty to cargo publish
  --no-verify        pass --no-verify (skips the packaged-crate build)
  --test             run the test suite as part of preflight
  --skip-data-check  do not compare crate data/ against espeak-ng-data/
  --assume-drained   pace as if the rate-limit buckets are empty
  --max-wait SECS    cap on a single 429 sleep (default 5400)

Rate limits (env or flag):
  --new-burst N --new-rate SECS --existing-burst N --existing-rate SECS
USAGE
}

while [ $# -gt 0 ]; do
    case "$1" in
        plan|package|publish) MODE="$1" ;;
        --only)            ONLY="$2"; shift ;;
        --exclude)         EXCLUDE="$2"; shift ;;
        --from)            FROM="$2"; shift ;;
        --limit)           LIMIT="$2"; shift ;;
        --no-main)         NO_MAIN=1 ;;
        -y|--yes)          ASSUME_YES=1 ;;
        --allow-dirty)     ALLOW_DIRTY=1 ;;
        --no-verify)       NO_VERIFY=1 ;;
        --test)            SKIP_TESTS=0 ;;
        --skip-data-check) SKIP_DATA_CHECK=1 ;;
        --assume-drained)  ASSUME_FULL=0 ;;
        --max-wait)        MAX_WAIT="$2"; shift ;;
        --new-burst)       NEW_BURST="$2"; shift ;;
        --new-rate)        NEW_RATE="$2"; shift ;;
        --existing-burst)  EXISTING_BURST="$2"; shift ;;
        --existing-rate)   EXISTING_RATE="$2"; shift ;;
        -h|--help)         usage; exit 0 ;;
        *) echo "unknown argument: $1" >&2; echo "try --help" >&2; exit 2 ;;
    esac
    shift
done

# ------------------------------------------------------------------ output --

if [ -t 1 ]; then
    B=$(printf '\033[1m'); R=$(printf '\033[0m')
    RED=$(printf '\033[31m'); GRN=$(printf '\033[32m'); YEL=$(printf '\033[33m')
    DIM=$(printf '\033[2m')
else
    B=""; R=""; RED=""; GRN=""; YEL=""; DIM=""
fi

log()  { printf '%s\n' "$*"; [ -n "$LOG_FILE" ] && printf '[%s] %s\n' "$(date -u +%H:%M:%S)" "$*" >>"$LOG_FILE"; return 0; }
info() { log "${DIM}$*${R}"; }
ok()   { log "${GRN}ok${R}   $*"; }
warn() { log "${YEL}warn${R} $*"; }
err()  { log "${RED}err${R}  $*"; }
die()  { err "$*"; exit 1; }

now() { date +%s; }

hms() {
    local s=$1
    if [ "$s" -lt 60 ]; then printf '%ds' "$s"
    elif [ "$s" -lt 3600 ]; then printf '%dm%02ds' $((s / 60)) $((s % 60))
    else printf '%dh%02dm' $((s / 3600)) $(((s % 3600) / 60))
    fi
}

mb() { awk -v b="$1" 'BEGIN { printf "%.2f MB", b / 1048576 }'; }

# Sleep until an absolute epoch, printing a countdown that overwrites itself.
sleep_until() {
    local target=$1 reason=$2 remaining
    while :; do
        remaining=$((target - $(now)))
        [ "$remaining" -le 0 ] && break
        if [ -t 1 ]; then
            printf '\r     %s%s, %s remaining ...%s\033[K' "$DIM" "$reason" "$(hms "$remaining")" "$R"
        fi
        if [ "$remaining" -gt 15 ]; then sleep 15; else sleep "$remaining"; fi
    done
    [ -t 1 ] && printf '\r\033[K'
    return 0
}

confirm() {
    [ "$ASSUME_YES" -eq 1 ] && return 0
    printf '%s' "$1 [y/N] "
    local reply; read -r reply
    case "$reply" in y|Y|yes|YES) return 0 ;; *) return 1 ;; esac
}

# ----------------------------------------------------------- crate listing --

# Publish order is dependency order: the phoneme tables first (every language
# crate is useless without them), then the per-language dictionaries, then the
# aggregates that re-export them, then the library itself.
crate_list() {
    cargo metadata --format-version 1 --no-deps 2>/dev/null </dev/null \
        | python3 -c '
import json, sys
pkgs = {p["name"]: p["version"] for p in json.load(sys.stdin)["packages"]}
versions = dict(pkgs)
order = []
def take(name):
    if name in pkgs:
        order.append(name)
        del pkgs[name]
take("espeak-ng-data-phonemes")
for name in sorted(k for k in pkgs if k.startswith("espeak-ng-data-dict-")):
    take(name)
take("espeak-ng-data-dicts")
main = pkgs.pop("espeak-ng", None)
order.extend(sorted(pkgs))          # anything unforeseen, before the library
if main is not None:
    order.append("espeak-ng")
for name in order:
    print("%s\t%s" % (name, versions[name]))
' || die "cargo metadata failed"
}

# --------------------------------------------------------------- the index --

INDEX_CACHE=$(mktemp -d "${TMPDIR:-/tmp}/publish-index.XXXXXX")
trap 'rm -rf "$INDEX_CACHE"' EXIT

# crates.io sparse-index layout: 1/x, 2/xy, 3/x/xyz, else xx/yy/name.
index_path() {
    local n=$1
    case ${#n} in
        1) printf '1/%s' "$n" ;;
        2) printf '2/%s' "$n" ;;
        3) printf '3/%s/%s' "${n:0:1}" "$n" ;;
        *) printf '%s/%s/%s' "${n:0:2}" "${n:2:2}" "$n" ;;
    esac
}

# Fetch a crate's index entry. Empty output means the crate does not exist
# yet; a non-zero return means the fetch itself failed.
index_fetch() {
    local name=$1 cache="$INDEX_CACHE/$1" code
    if [ -f "$cache" ]; then cat "$cache"; return 0; fi
    code=$(curl -sS -o "$cache.body" -w '%{http_code}' --max-time 30 \
        "https://index.crates.io/$(index_path "$name")" 2>/dev/null) || {
        rm -f "$cache.body"; return 1
    }
    case "$code" in
        200) mv "$cache.body" "$cache" ;;
        404) rm -f "$cache.body"; : >"$cache" ;;
        *)   rm -f "$cache.body"; return 1 ;;
    esac
    cat "$cache"
}

index_forget() { rm -f "$INDEX_CACHE/$1"; }

# Is this exact version already on the index? Yanked versions count: the
# version number is spent either way, so it can never be re-uploaded.
is_published() {
    local name=$1 version=$2 body
    body=$(index_fetch "$name") || return 2
    printf '%s' "$body" | grep -qF "\"vers\":\"$version\"" && return 0
    return 1
}

crate_is_new() {
    local body
    body=$(index_fetch "$1") || return 2
    [ -z "$body" ]
}

# After publishing, the dependent crates cannot resolve until the CDN in front
# of the index catches up. Poll rather than guess.
wait_for_index() {
    local name=$1 version=$2 deadline=$(( $(now) + INDEX_TIMEOUT ))
    while [ "$(now)" -lt "$deadline" ]; do
        index_forget "$name"
        if is_published "$name" "$version"; then return 0; fi
        sleep 5
    done
    return 1
}

# ------------------------------------------------------------- the limiter --

# GCRA ("leaky bucket as a meter"). TAT is the theoretical arrival time of the
# next request; holding it back-dated by burst*rate is what lets an idle bucket
# spend its whole burst at once. Each publish pushes TAT forward by one rate
# interval, so the sustained throughput settles at exactly 1/rate.
TAT_NEW=0
TAT_EXISTING=0

limiter_init() {
    local t; t=$(now)
    if [ "$ASSUME_FULL" -eq 1 ]; then
        TAT_NEW=$((t - NEW_BURST * NEW_RATE))
        TAT_EXISTING=$((t - EXISTING_BURST * EXISTING_RATE))
    else
        TAT_NEW=$t
        TAT_EXISTING=$t
    fi
}

# throttle new|existing - block until the bucket allows one more publish.
throttle() {
    local kind=$1 tat rate burst t earliest
    if [ "$kind" = new ]; then
        tat=$TAT_NEW; rate=$NEW_RATE; burst=$NEW_BURST
    else
        tat=$TAT_EXISTING; rate=$EXISTING_RATE; burst=$EXISTING_BURST
    fi

    t=$(now)
    [ "$tat" -lt "$t" ] && tat=$t
    earliest=$((tat - (burst - 1) * rate))
    if [ "$earliest" -gt "$t" ]; then
        sleep_until "$earliest" "rate limit ($kind crates: 1 per $(hms "$rate"))"
        t=$earliest
    fi
    tat=$((tat + rate))

    if [ "$kind" = new ]; then TAT_NEW=$tat; else TAT_EXISTING=$tat; fi
}

# Simulate the limiter over a pending list to produce an up-front ETA.
estimate_eta() {
    local pending_file=$1 kind
    python3 - "$pending_file" "$NEW_BURST" "$NEW_RATE" "$EXISTING_BURST" \
              "$EXISTING_RATE" "$EST_PUBLISH_SECS" "$ASSUME_FULL" <<'PY'
import sys
path, nb, nr, eb, er, est, full = sys.argv[1:8]
nb, nr, eb, er, est, full = int(nb), int(nr), int(eb), int(er), int(est), int(full)
clock = 0
tat = {"new": -nb * nr if full else 0, "existing": -eb * er if full else 0}
cfg = {"new": (nb, nr), "existing": (eb, er)}
for line in open(path):
    parts = line.split("\t")
    if len(parts) < 3:
        continue
    kind = parts[2].strip()
    burst, rate = cfg[kind]
    t = max(tat[kind], clock)
    earliest = t - (burst - 1) * rate
    if earliest > clock:
        clock = earliest
    tat[kind] = t + rate
    clock += est
print(clock)
PY
}

# ------------------------------------------------------------------- 429s ---

# crates.io says: "You have published too many crates in a short period of
# time. Please try again after Fri, 05 Sep 2026 06:59:09 GMT". Sleeping until
# that exact instant beats any backoff curve we could invent.
retry_after_epoch() {
    local stamp
    stamp=$(printf '%s' "$1" | sed -n 's/.*[Tt]ry again after \([A-Za-z]\{3\},* [0-9]\{1,2\} [A-Za-z]\{3\} [0-9]\{4\} [0-9:]\{8\} GMT\).*/\1/p' | head -1)
    [ -z "$stamp" ] && return 1
    # GNU date, then BSD date, then python.
    date -u -d "$stamp" +%s 2>/dev/null && return 0
    date -j -u -f '%a, %d %b %Y %H:%M:%S GMT' "$stamp" +%s 2>/dev/null && return 0
    python3 -c '
import sys, email.utils
t = email.utils.parsedate_to_datetime(sys.argv[1])
print(int(t.timestamp()))' "$stamp" 2>/dev/null && return 0
    return 1
}

is_rate_limited() {
    printf '%s' "$1" | grep -qiE '429|too many|rate limit|try again after'
}

# crates.io sits behind a CDN, so a failed upload is often nothing to do with
# the crate: a 5xx, a reset connection, a timeout. These are worth retrying.
is_transient() {
    printf '%s' "$1" | grep -qiE '50[0234]|backend (write|read|fetch) error|bad gateway|service unavailable|gateway time-?out|connection (reset|closed|refused)|timed out|temporary failure|EOF while'
}

# crates.io rejects a .crate over 10 MB. The CDN reports the refusal as a 503
# "backend write error" rather than a clean 413, so without this check the
# retry loop would spend its whole budget re-uploading something that can
# never be accepted.
is_too_large() {
    printf '%s' "$1" | grep -qiE 'too large|exceeds the maximum|max upload size|413'
}

already_published() {
    printf '%s' "$1" | grep -qiE 'already (exists|uploaded|published)|crate version .* is already|cannot be published'
}

# ------------------------------------------------------------- preflight ----

preflight_failed=0
pf_fail() { err "$*"; preflight_failed=$((preflight_failed + 1)); }

check_git() {
    git rev-parse --git-dir >/dev/null 2>&1 || { warn "not a git repository"; return 0; }
    if [ -n "$(git status --porcelain)" ]; then
        local n; n=$(git status --porcelain | wc -l | tr -d ' ')
        if [ "$ALLOW_DIRTY" -eq 1 ]; then
            warn "working tree is dirty ($n paths); publishing anyway (--allow-dirty)"
        else
            pf_fail "working tree is dirty ($n paths). Commit, or pass --allow-dirty."
        fi
    else
        ok "working tree is clean"
    fi
}

check_patch_section() {
    if grep -q '^\[patch\.crates-io\]' Cargo.toml; then
        pf_fail "[patch.crates-io] is present in Cargo.toml; remove it before publishing"
    else
        ok "no [patch.crates-io] override"
    fi
}

check_credentials() {
    if [ -n "${CARGO_REGISTRY_TOKEN:-}" ]; then
        ok "registry token from CARGO_REGISTRY_TOKEN"
    elif [ -f "${CARGO_HOME:-$HOME/.cargo}/credentials.toml" ] \
      || [ -f "${CARGO_HOME:-$HOME/.cargo}/credentials" ]; then
        ok "registry token from cargo credentials file"
    else
        pf_fail "no crates.io credentials; run 'cargo login' or set CARGO_REGISTRY_TOKEN"
    fi
}

# The data crates are snapshots of espeak-ng-data/. If a snapshot has drifted
# from its source, publishing ships a dictionary that does not match the
# library that reads it - and because a drifted crate usually still carries its
# old version number, it would otherwise be skipped as "already published"
# and the drift would reach users silently. This check is the reason that
# cannot happen.
check_data_freshness() {
    [ "$SKIP_DATA_CHECK" -eq 1 ] && { info "data freshness check skipped"; return 0; }
    [ -d espeak-ng-data ] || { warn "espeak-ng-data/ not found; skipping freshness check"; return 0; }

    local stale_crates=0 dir crate rel src n
    for dir in data-crates/*/; do
        [ -d "$dir/data" ] || continue
        crate=$(sed -n 's/^name[[:space:]]*=[[:space:]]*"\(.*\)".*/\1/p' "$dir/Cargo.toml" | head -1)
        n=0
        while IFS= read -r rel; do
            src="espeak-ng-data/$rel"
            if [ ! -f "$src" ] || ! cmp -s "$dir/data/$rel" "$src"; then
                n=$((n + 1))
            fi
        done < <(cd "$dir/data" && find . -type f | sed 's|^\./||')
        if [ "$n" -gt 0 ]; then
            stale_crates=$((stale_crates + 1))
            pf_fail "$crate: $n file(s) differ from espeak-ng-data/ - regenerate before publishing"
        fi
    done
    [ "$stale_crates" -eq 0 ] && ok "all data crates match espeak-ng-data/"
    return 0
}

check_tests() {
    [ "$SKIP_TESTS" -eq 1 ] && { info "test suite skipped (pass --test to run it)"; return 0; }
    log "running cargo test ..."
    if cargo test --quiet >/dev/null 2>&1 </dev/null; then ok "cargo test"
    else pf_fail "cargo test failed"; fi
}

# ----------------------------------------------------------------- publish --

record_state() {
    mkdir -p "$(dirname "$STATE_FILE")"
    printf '%s\t%s\t%s\t%s\n' "$(date -u +%FT%TZ)" "$1" "$2" "$3" >>"$STATE_FILE"
}

publish_one() {
    local name=$1 version=$2 kind=$3 attempt=1 out rc target

    local cmd=(cargo publish -p "$name")
    [ "$ALLOW_DIRTY" -eq 1 ] && cmd+=(--allow-dirty)
    [ "$NO_VERIFY" -eq 1 ] && cmd+=(--no-verify)
    [ -n "${CARGO_PUBLISH_EXTRA:-}" ] && cmd+=($CARGO_PUBLISH_EXTRA)

    while [ "$attempt" -le "$MAX_RETRIES" ]; do
        throttle "$kind"
        [ "$attempt" -gt 1 ] && info "retry $attempt/$MAX_RETRIES"
        log "  \$ ${cmd[*]}"

        set +e
        out=$("${cmd[@]}" 2>&1 </dev/null)
        rc=$?
        set -e
        [ -n "$LOG_FILE" ] && printf '%s\n' "$out" >>"$LOG_FILE"

        if [ "$rc" -eq 0 ]; then
            record_state "$name" "$version" published
            return 0
        fi

        if already_published "$out"; then
            warn "$name $version was already published; continuing"
            record_state "$name" "$version" already-published
            return 0
        fi

        if is_rate_limited "$out"; then
            if target=$(retry_after_epoch "$out"); then
                local wait=$((target - $(now)))
                if [ "$wait" -gt "$MAX_WAIT" ]; then
                    printf '%s\n' "$out" >&2
                    die "rate limited for $(hms "$wait"), over the --max-wait cap of $(hms "$MAX_WAIT")"
                fi
                warn "rate limited; crates.io says retry at $(date -u -r "$target" +%H:%M:%SZ 2>/dev/null || echo "$target")"
                sleep_until "$((target + 5))" "server-imposed cooldown"
            else
                local wait=$((attempt * attempt * 60))
                warn "rate limited with no parseable timestamp; backing off $(hms "$wait")"
                sleep_until "$(( $(now) + wait ))" "backoff"
            fi
            # A 429 means nothing was consumed, so do not let the local
            # limiter double-charge for this attempt.
            if [ "$kind" = new ]; then TAT_NEW=$((TAT_NEW - NEW_RATE))
            else TAT_EXISTING=$((TAT_EXISTING - EXISTING_RATE)); fi
            attempt=$((attempt + 1))
            continue
        fi

        if is_too_large "$out"; then
            printf '%s\n' "$out" >&2
            err "$name $version exceeds the 10 MB crates.io limit; retrying cannot help."
            err "Run '$0 package --only \"^$name\$\"' to see the packaged size, then"
            err "add the offending paths to that crate's exclude/include list."
            record_state "$name" "$version" too-large
            return 1
        fi

        if is_transient "$out"; then
            local wait=$((attempt * 30))
            warn "transient registry failure; retrying in $(hms "$wait")"
            printf '%s\n' "$out" | grep -iE '50[0234]|error' | head -3 >&2
            sleep_until "$(( $(now) + wait ))" "waiting out a registry error"
            # Nothing was consumed, so do not let the limiter charge for it.
            if [ "$kind" = new ]; then TAT_NEW=$((TAT_NEW - NEW_RATE))
            else TAT_EXISTING=$((TAT_EXISTING - EXISTING_RATE)); fi
            attempt=$((attempt + 1))
            continue
        fi

        printf '%s\n' "$out" >&2
        record_state "$name" "$version" failed
        return 1
    done

    record_state "$name" "$version" gave-up
    return 1
}

MAX_CRATE_BYTES=${MAX_CRATE_BYTES:-10485760}   # crates.io hard limit

package_one() {
    local name=$1 version=$2 out rc crate bytes
    log "  \$ cargo package -p $name --allow-dirty"
    set +e
    out=$(cargo package -p "$name" --allow-dirty 2>&1 </dev/null)
    rc=$?
    set -e
    [ -n "$LOG_FILE" ] && printf '%s\n' "$out" >>"$LOG_FILE"
    if [ "$rc" -ne 0 ]; then
        printf '%s\n' "$out" >&2
        return 1
    fi

    # Checking the size here is the whole point of packaging before publishing:
    # crates.io refuses an oversized upload through the CDN as an opaque 503.
    crate="$ROOT/target/package/$name-$version.crate"
    if [ -f "$crate" ]; then
        bytes=$(stat -f%z "$crate" 2>/dev/null || stat -c%s "$crate" 2>/dev/null || echo 0)
        if [ "$bytes" -gt "$MAX_CRATE_BYTES" ]; then
            err "$name $version packages to $(mb "$bytes"), over the $(mb "$MAX_CRATE_BYTES") crates.io limit"
            err "  largest paths:"
            tar tzvf "$crate" 2>/dev/null | sort -k3 -rn | head -5 | sed 's/^/      /' >&2
            return 1
        fi
        info "    $(mb "$bytes")"
    fi
    return 0
}

# -------------------------------------------------------------------- main --

if [ "$MODE" = publish ]; then
    mkdir -p "$ROOT/target/publish-logs"
    LOG_FILE="$ROOT/target/publish-logs/$(date -u +%Y%m%dT%H%M%SZ).log"
fi

log "${B}espeak-ng-rs publish${R}  (mode: $MODE)"
log ""
log "${B}Preflight${R}"
check_git
check_patch_section
[ "$MODE" = publish ] && check_credentials
check_data_freshness
check_tests
log ""

if [ "$preflight_failed" -gt 0 ] && [ "$MODE" = publish ]; then
    die "$preflight_failed preflight check(s) failed; refusing to publish"
fi

# Build the work list, classifying each crate as new / existing / done.
log "${B}Resolving crates against the index${R}"
PENDING=$(mktemp "${TMPDIR:-/tmp}/publish-pending.XXXXXX")
SKIPPED=0
TOTAL=0
started=0

while IFS=$'\t' read -r name version; do
    [ -z "$name" ] && continue
    [ "$NO_MAIN" -eq 1 ] && [ "$name" = espeak-ng ] && continue
    [ -n "$ONLY" ] && ! printf '%s' "$name" | grep -qE "$ONLY" && continue
    [ -n "$EXCLUDE" ] && printf '%s' "$name" | grep -qE "$EXCLUDE" && continue
    if [ -n "$FROM" ] && [ "$started" -eq 0 ]; then
        [ "$name" = "$FROM" ] && started=1 || continue
    fi
    TOTAL=$((TOTAL + 1))

    if is_published "$name" "$version"; then
        SKIPPED=$((SKIPPED + 1))
        continue
    fi
    if crate_is_new "$name"; then kind=new; else kind=existing; fi
    printf '%s\t%s\t%s\n' "$name" "$version" "$kind" >>"$PENDING"
done <<EOF
$(crate_list)
EOF

NEW_COUNT=$(grep -c '	new$' "$PENDING" 2>/dev/null || true)
EXISTING_COUNT=$(grep -c '	existing$' "$PENDING" 2>/dev/null || true)
NEW_COUNT=${NEW_COUNT:-0}; EXISTING_COUNT=${EXISTING_COUNT:-0}
PENDING_COUNT=$((NEW_COUNT + EXISTING_COUNT))

log ""
log "${B}Plan${R}"
log "  $TOTAL crate(s) in scope"
log "  $SKIPPED already on crates.io at the local version (skipped)"
log "  $NEW_COUNT new crate(s), $EXISTING_COUNT new version(s) of existing crates"

if [ "$PENDING_COUNT" -eq 0 ]; then
    log ""
    ok "nothing to publish"
    rm -f "$PENDING"
    exit 0
fi

log ""
i=0
while IFS=$'\t' read -r name version kind; do
    i=$((i + 1))
    printf '  %3d. %-34s %-8s %s%s%s\n' "$i" "$name" "$version" "$DIM" "$kind" "$R"
done <"$PENDING"

ETA=$(estimate_eta "$PENDING")
log ""
log "  estimated wall time: ${B}$(hms "$ETA")${R} ${DIM}(rate limits + ~${EST_PUBLISH_SECS}s per crate)${R}"
[ "$NEW_COUNT" -gt "$NEW_BURST" ] && warn "$NEW_COUNT new crates exceeds the burst of $NEW_BURST; the tail paces at 1 per $(hms "$NEW_RATE")"

if [ "$MODE" = plan ]; then
    log ""
    info "plan only. Re-run with 'package' to verify builds, or 'publish' to upload."
    rm -f "$PENDING"
    exit 0
fi

if [ "$MODE" = package ]; then
    log ""
    log "${B}Packaging${R}"
    failed=0
    while IFS=$'\t' read -r name version kind; do
        log "- $name $version"
        if package_one "$name" "$version"; then ok "packaged $name"; else err "failed to package $name"; failed=$((failed + 1)); fi
    done <"$PENDING"
    rm -f "$PENDING"
    log ""
    [ "$failed" -eq 0 ] && { ok "all crates package cleanly"; exit 0; }
    die "$failed crate(s) failed to package"
fi

# ---- publish -----------------------------------------------------------------

log ""
if ! confirm "Publish $PENDING_COUNT crate(s) to crates.io? This cannot be undone."; then
    log "aborted"
    rm -f "$PENDING"
    exit 1
fi

log ""
log "${B}Publishing${R}  ${DIM}log: $LOG_FILE${R}"
limiter_init
START=$(now)
done_count=0
fail_count=0

while IFS=$'\t' read -r name version kind; do
    if [ "$LIMIT" -gt 0 ] && [ "$done_count" -ge "$LIMIT" ]; then
        warn "--limit $LIMIT reached; stopping. Resume with: $0 publish --from $name"
        break
    fi

    log ""
    log "${B}[$((done_count + fail_count + 1))/$PENDING_COUNT] $name $version${R} ${DIM}($kind)${R}"

    if publish_one "$name" "$version" "$kind"; then
        done_count=$((done_count + 1))
        if wait_for_index "$name" "$version"; then
            ok "$name $version live on the index"
        else
            warn "$name $version not visible on the index after $(hms "$INDEX_TIMEOUT"); later crates may fail to resolve"
        fi
    else
        fail_count=$((fail_count + 1))
        err "$name $version failed"
        log ""
        err "stopping. Fix the problem, then resume with:"
        err "    $0 publish --from $name"
        break
    fi
done <"$PENDING"

rm -f "$PENDING"
ELAPSED=$(( $(now) - START ))

log ""
log "${B}Summary${R}"
log "  published: $done_count"
log "  failed:    $fail_count"
log "  elapsed:   $(hms "$ELAPSED")"
log "  state log: $STATE_FILE"
[ "$fail_count" -eq 0 ] || exit 1
