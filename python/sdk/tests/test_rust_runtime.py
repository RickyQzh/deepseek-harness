"""Keyless boot of the Rust jsonrpc bin when present; skip if not built."""

from __future__ import annotations

from pathlib import Path

import pytest

from deepseek_harness import HarnessClient, HarnessConfig
from deepseek_harness_runtime import resolve_bundled_launch_args

_REPO_ROOT = Path(__file__).resolve().parents[3]
_MINIMAL_CONFIG = _REPO_ROOT / "crates" / "dsh-sdk-jsonrpc-server" / "minimal.cordis.yml"


def test_rust_runtime_initialize_server_info_name(tmp_path: Path) -> None:
    try:
        launch_args = resolve_bundled_launch_args("rust")
    except FileNotFoundError as exc:
        pytest.skip(f"Rust jsonrpc bin not built: {exc}")
    if not _MINIMAL_CONFIG.is_file():
        pytest.skip(f"missing {_MINIMAL_CONFIG}")
    sessions = tmp_path / "sessions"
    sessions.mkdir()
    client = HarnessClient(
        HarnessConfig(
            launch_args_override=launch_args,
            cwd=str(tmp_path),
            env={
                "DSH_CORDIS_CONFIG": str(_MINIMAL_CONFIG),
                "DSH_SESSION_ROOT": str(sessions),
                "DSH_CWD": str(tmp_path),
            },
            request_timeout_seconds=120,
        )
    )
    with client:
        init = client.initialize(
            provider="deepseek-official", cwd=str(tmp_path), model="deepseek-v4-flash"
        )
    assert init.serverInfo is not None
    assert init.serverInfo.name == "deepseek-harness-sdk-runtime"
