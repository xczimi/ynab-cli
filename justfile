# Development tasks for ynab-cli. Run `just` to list them.
# `just` is pinned in mise.toml — `mise install` provides it.

set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

# Everything CI runs, in CI's order, so green here means green there.
ci: fmt-check lint test audit

fmt:
    cargo fmt

fmt-check:
    cargo fmt --check

lint:
    cargo clippy --all-targets --locked -- -D warnings

test:
    cargo test --locked

audit:
    #!/usr/bin/env bash
    set -euo pipefail
    if ! command -v cargo-audit >/dev/null; then
        echo "cargo-audit is not installed: cargo install cargo-audit --locked" >&2
        exit 1
    fi
    cargo audit

# What has landed since the last release — the evidence for choosing a
# bump type. Deciding patch vs minor vs major is a judgement call; this
# recipe only shows you what you are deciding about.
[doc("Show commits since the last release tag")]
changes:
    #!/usr/bin/env bash
    set -euo pipefail
    last=$(git describe --tags --abbrev=0 2>/dev/null || echo "")
    if [[ -z "$last" ]]; then
        echo "no release tags yet; showing all commits"
        git log --oneline
        exit 0
    fi
    echo "since $last:"
    git log --oneline "$last..HEAD"

# Cut a release: bump the version, tag it, and let release.yml publish to
# crates.io. Usage: just release patch | minor | major | 0.1.3
#
# Refuses to run unless main is clean, synced, and green in CI. Set
# ALLOW_RED_CI=1 to release anyway.
[doc("Bump the version, tag it, and let CI publish to crates.io")]
release spec:
    #!/usr/bin/env bash
    set -euo pipefail

    spec="{{spec}}"
    if [[ ! "$spec" =~ ^(patch|minor|major)$ ]] && \
       [[ ! "$spec" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
        echo "expected patch, minor, major, or an explicit X.Y.Z — got '$spec'" >&2
        exit 1
    fi

    branch=$(git rev-parse --abbrev-ref HEAD)
    if [[ "$branch" != "main" ]]; then
        echo "on branch '$branch'; releases are cut from main" >&2
        exit 1
    fi

    if [[ -n "$(git status --porcelain)" ]]; then
        echo "working tree is dirty; commit or stash first" >&2
        exit 1
    fi

    git fetch --quiet origin main
    if [[ "$(git rev-parse HEAD)" != "$(git rev-parse origin/main)" ]]; then
        echo "main is not in sync with origin/main; push or pull first" >&2
        exit 1
    fi

    # A crates.io release cannot be undone, so it is only worth as much as
    # the signal on the commit it is cut from.
    sha=$(git rev-parse HEAD)
    conclusion=$(gh run list --workflow ci.yml --commit "$sha" --limit 1 \
        --json conclusion --jq '.[0].conclusion // "no run"')
    if [[ "$conclusion" != "success" ]]; then
        echo "CI on ${sha:0:7} is '$conclusion', not success." >&2
        echo "Look: gh run list --commit $sha" >&2
        echo "Override: ALLOW_RED_CI=1 just release $version" >&2
        [[ "${ALLOW_RED_CI:-}" == "1" ]] || exit 1
        echo "ALLOW_RED_CI=1 set, continuing anyway." >&2
    fi

    current=$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[0].version')
    IFS=. read -r cur_major cur_minor cur_patch <<< "$current"

    case "$spec" in
        patch) version="$cur_major.$cur_minor.$((cur_patch + 1))" ;;
        minor) version="$cur_major.$((cur_minor + 1)).0" ;;
        major)
            version="$((cur_major + 1)).0.0"
            # Cargo treats 0.1.x as compatible within 0.1, so while the crate
            # is 0.x the breaking bump is `minor` (0.1.2 -> 0.2.0). Warn
            # rather than silently remap: 1.0.0 is a real decision.
            if [[ "$cur_major" == "0" ]]; then
                echo "note: at $current a breaking change is 'minor' ($cur_major.$((cur_minor + 1)).0)." >&2
                echo "      'major' here means committing to $version. Ctrl-C within 5s to reconsider." >&2
                sleep 5
            fi
            ;;
        *) version="$spec" ;;
    esac
    tag="v$version"

    if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
        echo "tag $tag already exists" >&2
        exit 1
    fi

    echo "==> $current -> $version"

    # Portable in-place edit of the first `version = ` line (BSD sed lacks
    # the 0,/re/ address form).
    awk -v v="$version" '
        /^version = / && !done { print "version = \"" v "\""; done = 1; next }
        { print }
    ' Cargo.toml > Cargo.toml.tmp && mv Cargo.toml.tmp Cargo.toml

    cargo check --quiet   # refreshes the crate's own entry in Cargo.lock

    written=$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[0].version')
    if [[ "$written" != "$version" ]]; then
        echo "version bump did not take (Cargo.toml says $written)" >&2
        git checkout -- Cargo.toml Cargo.lock
        exit 1
    fi

    git add Cargo.toml Cargo.lock
    git commit -m "chore: release $tag"
    git push origin main

    git tag -a "$tag" -m "$tag"
    git push origin "$tag"

    gh release create "$tag" --title "$tag" --generate-notes

    echo
    echo "==> $tag pushed. release.yml is verifying and will publish to crates.io."
    echo "    gh run watch \$(gh run list --workflow release.yml --limit 1 --json databaseId --jq '.[0].databaseId')"
