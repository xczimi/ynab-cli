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

# Cut a release: bump the version, tag it, and let release.yml publish to
# crates.io. Usage: just release 0.1.3
#
# Refuses to run unless main is clean, synced, and green in CI. Set
# ALLOW_RED_CI=1 to release anyway.
[doc("Bump the version, tag it, and let CI publish to crates.io")]
release version:
    #!/usr/bin/env bash
    set -euo pipefail

    version="{{version}}"
    tag="v$version"

    if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
        echo "not a semver version: $version" >&2
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

    if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
        echo "tag $tag already exists" >&2
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
