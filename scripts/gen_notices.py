"""Generate THIRD-PARTY-NOTICES.txt in the repo root.

Lists IBM Plex Sans + JetBrains Mono (OFL), LAME via mp3lame-encoder
(LGPL-3.0) and the license of every crate in Cargo.lock, read from the
extracted crate sources in the local cargo registry (no network). Run from
the repo root: python scripts/gen_notices.py
"""
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LOCK = ROOT / "Cargo.lock"
OUT = ROOT / "THIRD-PARTY-NOTICES.txt"

# Local sparse-registry cache written by cargo; no network needed.
REGISTRY = Path.home() / ".cargo" / "registry" / "src" / "index.crates.io-1949cf8c6b5b557f"
INDEX = Path.home() / ".cargo" / "registry" / "index" / "index.crates.io-1949cf8c6b5b557f"

# Hand-written preamble: bundled assets and the C library behind
# mp3lame-encoder, whose crate license is covered by the crate list below.
PREAMBLE = """\
Third-party notices for Audio Editor
====================================

Fonts (bundled under assets/fonts, licenses in the same folder):

- IBM Plex Sans (assets/fonts/IBMPlexSans-*.ttf)
  License: SIL Open Font License 1.1 - assets/fonts/LICENSE-IBMPlex.txt
  Copyright (c) 2014 IBM Corp.

- JetBrains Mono (assets/fonts/JetBrainsMono-*.ttf)
  License: SIL Open Font License 1.1 - assets/fonts/OFL-JetBrainsMono.txt
  Copyright 2020 The JetBrains Mono Project Authors

- LAME MP3 encoder, used through the mp3lame-encoder crate
  License: LGPL-3.0 (https://www.gnu.org/licenses/lgpl-3.0.html)
  LAME is linked as a compiled C library (mp3lame-sys) and is not modified.

Rust crates (from Cargo.lock; license as declared on crates.io):
"""


def lock_packages() -> list[tuple[str, str]]:
    """Unique (name, version) pairs from Cargo.lock, sorted by name."""
    text = LOCK.read_text(encoding="utf-8")
    pkgs: dict[str, str] = {}
    for block in re.split(r"\[\[package\]\]", text)[1:]:
        name = re.search(r'^name = "(.+)"', block, re.M)
        version = re.search(r'^version = "(.+)"', block, re.M)
        if name and version:
            pkgs.setdefault(name.group(1), version.group(1))
    return sorted(pkgs.items())


def index_cache_path(name: str) -> Path:
    """Sparse-index shard path: ab_glyph -> .cache/ab/_g/ab_glyph."""
    if len(name) >= 4:
        shard = name[:2], name[2:4]
    elif len(name) == 3:
        shard = "3", name[2]
    else:
        shard = "2", name[1]
    return INDEX / ".cache" / shard[0] / shard[1] / name


def license_from_source(name: str, version: str) -> str | None:
    """License from the extracted crate's own Cargo.toml (offline)."""
    toml = REGISTRY / f"{name}-{version}" / "Cargo.toml"
    if not toml.exists():
        return None
    text = toml.read_text(encoding="utf-8", errors="replace")
    lic = re.search(r'(?m)^license\s*=\s*"([^"]+)"', text)
    if lic:
        return lic.group(1)
    if re.search(r"(?m)^license-file\s*=", text):
        return "license-file (see crate source)"
    return None


def main() -> None:
    pkgs = lock_packages()
    lines = [PREAMBLE]
    missing = 0
    for name, version in pkgs:
        if name == "audio_editor":  # the root crate itself, not a dependency
            continue
        lic = license_from_source(name, version)
        if lic is None:
            lic = "UNKNOWN (crate source not downloaded)"
            missing += 1
            print(f"  ! no license for {name} {version}")
        lines.append(f"- {name} {version} - {lic}")
    OUT.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"wrote {OUT} ({len(pkgs) - 1} crates, {missing} unknown)")


if __name__ == "__main__":
    main()
