#!/usr/bin/env bash
# Publishes the packages built by `npm/build.mjs` in the directory given: the programs of
# each system first, then `@hessbey/parfumo-mcp`, which depends on them. A package already
# published at this version is skipped, so that the job can be run again after a failure.
set -euo pipefail

dist="${1:?usage: publish.sh <directory of the built packages>}"
version=$(node -p "require('./$dist/parfumo-mcp/package.json').version")
# A pre-release (1.2.3-rc.1) does not become what `npx @hessbey/parfumo-mcp` installs.
tag=latest
case "$version" in *-*) tag=next ;; esac

publish() {
  local dir="$1" name
  name=$(node -p "require('./$dir/package.json').name")
  if npm view "$name@$version" version >/dev/null 2>&1; then
    echo "$name@$version is already published"
    return 0
  fi
  echo "Publishing $name@$version ($tag)"
  npm publish "./$dir" --access public --tag "$tag"
}

for dir in "$dist"/parfumo-mcp-* "$dist"/parfumo-mcp; do
  publish "$dir"
done

# The plain name `parfumo-mcp` is only kept so that nobody else takes it. npm's spam filter
# may refuse it, as it refused `parfumo-mcp-win32-x64`: that does not fail the release.
if ! publish "$dist"/unscoped; then
  echo "::warning::The plain name parfumo-mcp could not be published; @hessbey/parfumo-mcp is the package to use."
fi
