"""Where the runtime finds the project.

Mirrors the Rust side (`src/paths.rs`, sent as `SVS_HOME` when the app spawns
this process): `SVS_HOME` wins, otherwise the nearest ancestor of the working
directory that holds `config/app.toml`.

Depth-independent on purpose. The adapters used to derive this by counting
parent directories from `__file__`, which silently relocated every fallback
output directory the moment the tree was reorganised.
"""

from __future__ import annotations

import os
from pathlib import Path

CONFIG_FILE = Path("config") / "app.toml"


def project_root() -> Path:
    """The checkout (or bundle) root every self-resolved path hangs off."""
    configured = os.environ.get("SVS_HOME")
    if configured:
        return Path(configured).expanduser().resolve()
    working_directory = Path.cwd().resolve()
    for candidate in (working_directory, *working_directory.parents):
        if (candidate / CONFIG_FILE).is_file():
            return candidate
    return working_directory
