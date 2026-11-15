#!/bin/sh
# Writes the files for the Flathub repository of the app: the manifest,
# building a tagged commit of the public repository instead of this
# directory, and the list of crates for that commit's Cargo.lock.
#
#     packaging/flatpak/flathub.sh <tag> <commit> <out-dir>
#
# Run it in a checkout of that tag (Cargo.lock must be the tag's), for example:
#
#     packaging/flatpak/flathub.sh v0.1.0 "$(git rev-parse v0.1.0^{commit})" ../flathub-specto
#
# See packaging/README.md for the rest of publishing.
set -eu
[ $# -eq 3 ] || { sed -n '2,12s/^# \{0,1\}//p' "$0" >&2; exit 2; }
tag=$1 commit=$2 out=$3
url=https://github.com/xfbs/specto.git
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)

mkdir -p "$out"
python3 "$here/cargo-sources.py" "$root/Cargo.lock" "$out/cargo-sources.json"
# The local manifest, with its `dir` source (and what it skips) replaced.
awk -v url="$url" -v tag="$tag" -v commit="$commit" '
    /^# / && NR <= 2 { next }
    /^ *- type: dir$/ {
        print "      - type: git"
        print "        url: " url
        print "        tag: " tag
        print "        commit: \"" commit "\""
        skipping = 1
        next
    }
    skipping && /^ *- cargo-sources\.json$/ { skipping = 0 }
    !skipping
' "$here/net.xfbs.specto.yml" > "$out/net.xfbs.specto.yml"
grep -q "commit: \"$commit\"" "$out/net.xfbs.specto.yml" || { echo "the manifest has no dir source to replace" >&2; exit 1; }
echo "$out/net.xfbs.specto.yml"
echo "$out/cargo-sources.json"
