#!/usr/bin/env bash
# Fetch every download the page offers, and fail if one is not a file.
#
# **It changes its own mind about whether a broken link is fatal**, rather than
# being told by a flag someone has to remember to remove. There are no releases
# yet, so every `releases/latest/download/…` URL 404s — the names are correct
# (#189) and their destinations do not exist. Warning about that today is
# right; failing would make a red mark everyone learns to ignore, and a red
# mark everyone ignores is worse than no check.
#
# So the condition is asked rather than assumed: no release means warn, a
# release means any broken link is a failure. It is correct now, becomes strict
# the moment the first version is tagged, and nobody has to delete a line to
# make that happen. Human memory in the load-bearing position is the thing this
# repository keeps losing to.
#
# It is a redirect chain, so `--location` is required: GitHub answers
# `releases/latest/download/x` with a 302 to the tagged asset. Without it every
# URL "succeeds" with a 302 and the check is a decoration.
set -uo pipefail

# `gh` needs no token beyond the default read permission. If it cannot answer
# at all — no `gh`, no network — that is treated as "no release", because
# guessing "there is one" would fail a build over a missing tool.
if releases=$(gh release list --limit 1 2>/dev/null) && [ -n "$releases" ]; then
  strict=1
  echo "A release exists, so a broken download is a failure."
else
  strict=0
  echo "No release yet, so a broken download is a warning."
  echo "This becomes strict on its own when the first version is tagged."
fi
echo

base="https://github.com/clispeak/clispeak/releases/latest/download"
names=(
  clispeak-linux.flatpak
  clispeak-macos.dmg
  clispeak-windows-setup.exe
  clispeak-android.apk
)

fail=0
for n in "${names[@]}"; do
  url="$base/$n"
  # --head so a passing check does not download four installers, --location to
  # follow the redirect, and the effective URL printed so a wrong destination
  # is visible rather than merely a status.
  read -r code size final < <(
    curl --silent --show-error --head --location \
         --write-out '%{http_code} %{size_download} %{url_effective}' \
         --output /dev/null "$url" 2>/dev/null || echo "000 0 -"
  )
  if [ "$code" = "200" ]; then
    printf 'ok    %-30s %s\n' "$n" "$final"
  else
    printf 'FAIL  %-30s HTTP %s\n      %s\n' "$n" "$code" "$url"
    fail=1
  fi
done

if [ "$fail" -eq 0 ]; then
  exit 0
fi

echo
echo "One or more downloads the site offers do not exist."
if [ "$strict" -eq 1 ]; then
  echo "A release is published, so these should resolve. This is a failure."
  exit 1
fi
echo "No release is published yet, so this is expected — see the sequencing"
echo "section of docs/website.md. It will fail rather than warn once one is."
exit 0
