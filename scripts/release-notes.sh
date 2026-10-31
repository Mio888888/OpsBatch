#!/bin/sh
# Print the notes of a release as Markdown. Used by .github/workflows/release.yml for the GitHub release body and
# for the `notes` field of latest.json (what the update dialog of an installed app shows).
#
#   scripts/release-notes.sh <tag> [owner/repo]
#
# 1. release-notes/<tag>.md, when it exists, is used as written. Write one for a release that needs more than a
#    list of commits (the first release of a line, a migration, a breaking change).
# 2. Otherwise the commit subjects since the previous release tag that is reachable from HEAD are grouped by what
#    they say: performance, fixes, maintenance (docs, CI, formatting, packaging), and everything else as new
#    features and changes. Only the tags of this history count, so the Tauri tags (v0.x) are never picked up.
#    With no earlier tag the last 50 commits are listed.
set -e
[ "$#" -ge 1 ] || { echo "usage: release-notes.sh <tag> [owner/repo]" >&2; exit 2; }
tag="$1"
repo="${2:-$GITHUB_REPOSITORY}"
root=$(git rev-parse --show-toplevel)

if [ -f "$root/release-notes/$tag.md" ]; then
  cat "$root/release-notes/$tag.md"
  exit 0
fi

prev=$(git describe --tags --abbrev=0 --match 'v*' --exclude "$tag" HEAD 2>/dev/null || true)
if [ -n "$prev" ]; then
  subjects=$(git log --no-merges --format=%s "$prev..HEAD")
else
  subjects=$(git log --no-merges --format=%s -n 50 HEAD)
fi

feat=""
perf=""
fixes=""
maint=""
nl='
'
# $1 = current list, $2 = new line; prints the list with the line appended
append() { if [ -z "$1" ]; then printf '%s' "- $2"; else printf '%s%s%s' "$1" "$nl" "- $2"; fi; }

while IFS= read -r subject; do
  [ -n "$subject" ] || continue
  line=$(printf '%s' "$subject" | sed 's/[. ]*$//')
  lower=$(printf '%s' "$subject" | tr 'A-Z' 'a-z')
  case "$lower" in
    release\ v[0-9]*|bump\ *|chore\(release\)*|merge\ *) continue ;;
    *clippy*|*rustfmt*|*cargo\ fmt*|*format\ the\ workspace*|*workflow*|*readme*|*license*|*website*|*dependabot*|\
ci:*|ci\ *|docs:*|docs\ *|test:*|tests:*|chore*|deps*|*packaging*|*\ ci\ *|*github\ *)
      maint=$(append "$maint" "$line") ;;
    *pipelin*|*faster*|*speed*|*latency*|*smooth*|*perf*|*optimi*|*fluen*)
      perf=$(append "$perf" "$line") ;;
    fix*|*\ fix\ *|*\ fixes\ *|*bug*|*crash*|*regress*|*repair*|*broken*)
      fixes=$(append "$fixes" "$line") ;;
    *)
      feat=$(append "$feat" "$line") ;;
  esac
done <<SUBJECTS
$subjects
SUBJECTS

section() { [ -z "$2" ] || printf '## %s\n\n%s\n\n' "$1" "$2"; }
section "New features and changes / 新功能与改动" "$feat"
section "Performance / 性能" "$perf"
section "Fixes / 问题修复" "$fixes"
section "Maintenance / 优化与维护" "$maint"
if [ -z "$feat$perf$fixes$maint" ]; then
  printf 'See the assets to download this version.\n\n'
fi
if [ -n "$prev" ] && [ -n "$repo" ]; then
  printf '**Full changelog / 完整变更**: https://github.com/%s/compare/%s...%s\n' "$repo" "$prev" "$tag"
fi
