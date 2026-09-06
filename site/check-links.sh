#!/usr/bin/env bash
# Fetch every download the page offers, and fail if one is not a file.
#
# **This is expected to fail today, and that is it working.** There are no tags
# and no releases in this repository, so every `releases/latest/download/…`
# URL 404s. The names are correct (#189) and their destinations do not exist
# yet; the sequencing in docs/website.md says the release comes first and DNS
# comes last. A check that passed today would be checking nothing.
#
# It is a redirect chain, so `--location` is required: GitHub answers
# `releases/latest/download/x` with a 302 to the tagged asset. Without it every
# URL "succeeds" with a 302 and the check is a decoration.
set -uo pipefail

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

if [ "$fail" -ne 0 ]; then
  echo
  echo "One or more downloads the site offers do not exist."
  echo "Expected until a version is tagged and its release published —"
  echo "see the sequencing section of docs/website.md."
  exit 1
fi
