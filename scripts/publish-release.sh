#!/usr/bin/env bash
set -euo pipefail
: "${GH_REPO:?}"
: "${RELEASE_TAG:?}"
: "${RELEASE_VERSION:?}"
: "${RELEASE_COMMIT:?}"

if [[ "$RELEASE_TAG" != "v$RELEASE_VERSION" ]] || [[ ! "$RELEASE_TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
    echo 'Invalid release tag.' >&2
    exit 1
fi
if [[ ! "$RELEASE_COMMIT" =~ ^[0-9a-f]{40}$ ]]; then
    echo 'Invalid source commit.' >&2
    exit 1
fi
notes="docs/releases/$RELEASE_TAG.md"
test -f "$notes"
files=(
    "DroidWebcam-$RELEASE_VERSION-windows-x86_64.zip"
    "DroidWebcam-$RELEASE_VERSION-ubuntu-x86_64.tar.gz"
    "DroidWebcam-$RELEASE_VERSION-android.apk"
)
for file in "${files[@]}"; do test -s "release/$file"; done
(cd release && sha256sum "${files[@]}" > SHA256SUMS.txt && sha256sum --check SHA256SUMS.txt)

if git show-ref --verify --quiet "refs/tags/$RELEASE_TAG"; then
    if [[ "$(git rev-parse "$RELEASE_TAG^{commit}")" != "$RELEASE_COMMIT" ]]; then
        echo 'The existing release tag points to a different commit.' >&2
        exit 1
    fi
else
    gh api --method POST "repos/$GH_REPO/git/refs" -f "ref=refs/tags/$RELEASE_TAG" -f "sha=$RELEASE_COMMIT" >/dev/null
fi

title="Droid Webcam $RELEASE_TAG Preview"
if draft=$(gh release view "$RELEASE_TAG" --repo "$GH_REPO" --json isDraft --jq '.isDraft' 2>/dev/null); then
    if [[ "$draft" == 'false' ]]; then
        echo "Release $RELEASE_TAG is already published at the same source commit."
        exit 0
    fi
else
    gh release create "$RELEASE_TAG" --repo "$GH_REPO" --verify-tag --draft --prerelease --title "$title" --notes-file "$notes"
fi
# Keep a retryable draft until all downloads and their checksums have been attached.
gh release upload "$RELEASE_TAG" --repo "$GH_REPO" --clobber release/*
gh release edit "$RELEASE_TAG" --repo "$GH_REPO" --draft=false --prerelease --title "$title" --notes-file "$notes"
printf 'Published [%s](https://github.com/%s/releases/tag/%s) from `%s`.\n' "$RELEASE_TAG" "$GH_REPO" "$RELEASE_TAG" "$RELEASE_COMMIT" >> "$GITHUB_STEP_SUMMARY"
