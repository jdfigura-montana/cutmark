"""Generate THIRD-PARTY-NOTICES.txt in apps/audio-editor.

Lists Inter + JetBrains Mono (OFL) + Phosphor icons (MIT), the FFmpeg shared libraries
(LGPL-3.0-or-later) and the license of every crate in the cutmark-audio dependency
tree for Windows (via cargo tree), read from the extracted crate sources
in the local cargo registry (no network). Run from the repo root:
python apps\audio-editor\scripts\gen_notices.py
"""
import json
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "THIRD-PARTY-NOTICES.txt"

# Local sparse-registry cache written by cargo; no network needed.
REGISTRY = Path.home() / ".cargo" / "registry" / "src" / "index.crates.io-1949cf8c6b5b557f"
INDEX = Path.home() / ".cargo" / "registry" / "index" / "index.crates.io-1949cf8c6b5b557f"

# Hand-written preamble: bundled assets and the FFmpeg shared libraries,
# whose crate license is covered by the crate list below.
PREAMBLE = """\
Third-party notices for CutMark Audio Editor
====================================

Fonts and icons (embedded in the exe by the ui-kit crate):

- Inter (crates/ui-kit/assets/fonts/Inter-*.ttf)
  License: SIL Open Font License 1.1 - crates/ui-kit/assets/fonts/OFL-Inter.txt
  Copyright (c) 2016 The Inter Project Authors

- JetBrains Mono (crates/ui-kit/assets/fonts/JetBrainsMono-*.ttf)
  License: SIL Open Font License 1.1 - crates/ui-kit/assets/fonts/OFL-JetBrainsMono.txt
  Copyright 2020 The JetBrains Mono Project Authors

- Phosphor Icons, used through the egui-phosphor crate
  License: MIT - licenses/Phosphor-MIT.txt
  Copyright (c) 2020 Phosphor Icons

- FFmpeg 9.0.2 shared libraries (avcodec-63.dll, avdevice-63.dll, avfilter-12.dll,
  avformat-63.dll, avutil-61.dll, swresample-7.dll, swscale-10.dll)
  License: LGPL-3.0-or-later (https://www.gnu.org/licenses/lgpl-3.0.html)
  Linked dynamically as separate DLL files beside the exe that the user may
  replace; build details and source location in licenses/FFmpeg-BUILDINFO.txt.

Rust crates (from the cutmark-audio dependency tree; license as declared on crates.io):
"""


def tree_packages() -> list[tuple[str, str]]:
    """Unique (name, version) pairs in the cutmark-audio dependency tree on Windows, sorted by name."""
    out = subprocess.run(
        ["cargo", "tree", "-p", "cutmark-audio", "-e", "normal", "--target", "x86_64-pc-windows-msvc",
         "--prefix", "none", "--format", "{p}", "--offline"],
        cwd=ROOT, capture_output=True, text=True, check=True,
    ).stdout
    pkgs: dict[str, str] = {}
    for line in out.splitlines():
        m = re.match(r"^(\S+) v(\S+)", line)
        if m:
            pkgs.setdefault(m.group(1), m.group(2))
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
    pkgs = tree_packages()
    lines = [PREAMBLE]
    missing = 0
    for name, version in pkgs:
        if name == "cutmark-audio":  # the crate itself, not a dependency
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
