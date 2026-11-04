#!/usr/bin/env bash
# Decides whether a commit should produce a release, and writes the answer to GITHUB_OUTPUT.
#
# Usage: should-release.sh <event name> <previous commit>
#
# A push releases only when it raises the version in the root Cargo.toml. A manual run releases
# the current version, unless it has already been released.
set -euo pipefail

event=$1
previous_commit=${2:-}

version_at() {
  git show "$1:Cargo.toml" 2>/dev/null |
    awk '/^\[workspace\.package\]/ { found = 1; next }
         /^\[/ { found = 0 }
         found && /^version *=/ { gsub(/.*= *"|".*/, ""); print; exit }' || true
}

decide() {
  echo "should_release=$1" >>"${GITHUB_OUTPUT:-/dev/null}"
  echo "version=$version" >>"${GITHUB_OUTPUT:-/dev/null}"
  echo "should_release=$1 version=$version: $2"
  exit 0
}

version=$(version_at HEAD)
if [[ -z "$version" ]]; then
  echo "No version found under [workspace.package] in Cargo.toml" >&2
  exit 1
fi

if git ls-remote --exit-code --tags origin "refs/tags/v$version" >/dev/null; then
  decide false "v$version has already been released"
fi

if [[ "$event" == "workflow_dispatch" ]]; then
  decide true "requested manually"
fi

previous=$(version_at "$previous_commit")
if [[ -z "$previous" ]]; then
  decide false "no earlier workspace version to compare with"
fi
if [[ "$previous" == "$version" ]]; then
  decide false "version is unchanged"
fi
if [[ "$(printf '%s\n%s\n' "$previous" "$version" | sort -V | tail -n 1)" != "$version" ]]; then
  decide false "version went down from $previous"
fi
decide true "version went up from $previous"
