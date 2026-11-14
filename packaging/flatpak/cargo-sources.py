#!/usr/bin/env python3
"""Writes the Flatpak sources for the crates in Cargo.lock.

Flatpak builds have no network, so every crate is downloaded by
flatpak-builder beforehand and Cargo is pointed at them as a vendored
source. Like flatpak-cargo-generator from flatpak-builder-tools, but for
crates.io crates only (all this workspace uses) and without dependencies.

    cargo-sources.py Cargo.lock cargo-sources.json
"""

import json
import sys
import tomllib

CRATES_IO = "registry+https://github.com/rust-lang/crates.io-index"
VENDOR = "cargo/vendor"


def main(lock_path, out_path):
    with open(lock_path, "rb") as f:
        lock = tomllib.load(f)
    sources = []
    for package in lock["package"]:
        source = package.get("source")
        if source is None:
            continue  # part of the workspace
        if source != CRATES_IO:
            sys.exit(f"{package['name']}: only crates.io crates are supported, not {source}")
        name, version, checksum = package["name"], package["version"], package["checksum"]
        dest = f"{VENDOR}/{name}-{version}"
        sources.append({
            "type": "archive",
            "archive-type": "tar-gzip",
            "url": f"https://static.crates.io/crates/{name}/{name}-{version}.crate",
            "sha256": checksum,
            "dest": dest,
        })
        sources.append({
            "type": "inline",
            "contents": json.dumps({"package": checksum, "files": {}}),
            "dest": dest,
            "dest-filename": ".cargo-checksum.json",
        })
    sources.append({
        "type": "inline",
        "contents": (
            '[source.crates-io]\nreplace-with = "vendored-sources"\n\n'
            f'[source.vendored-sources]\ndirectory = "{VENDOR}"\n'
        ),
        "dest": "cargo",
        "dest-filename": "config.toml",
    })
    with open(out_path, "w") as f:
        json.dump(sources, f, indent=1)
        f.write("\n")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__.strip())
    main(sys.argv[1], sys.argv[2])
