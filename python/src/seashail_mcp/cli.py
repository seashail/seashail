import os
import shutil
import subprocess
import sys


def _run(cmd: list[str]) -> int:
    p = subprocess.run(cmd)
    return int(p.returncode)


def _resolve_seashail() -> str | None:
    # Prefer explicit override.
    p = os.environ.get("SEASHAIL_BIN")
    if p and os.path.isfile(p) and os.access(p, os.X_OK):
        return p

    # Normal PATH lookup.
    p = shutil.which("seashail")
    if p:
        return p

    # Common install locations from the hosted installer / cargo.
    home = os.path.expanduser("~")
    if sys.platform.startswith("win"):
        candidates = [
            os.path.join(home, ".local", "bin", "seashail.exe"),
            os.path.join(home, ".cargo", "bin", "seashail.exe"),
        ]
    else:
        candidates = [
            os.path.join(home, ".local", "bin", "seashail"),
            os.path.join(home, ".cargo", "bin", "seashail"),
        ]
    for c in candidates:
        if os.path.isfile(c) and os.access(c, os.X_OK):
            return c

    return None


def _resolve_powershell() -> str:
    p = shutil.which("pwsh")
    if p:
        return p
    p = shutil.which("powershell")
    if p:
        return p
    raise RuntimeError("missing dependency: powershell (or pwsh)")


def _install_from_source() -> None:
    if sys.platform.startswith("win"):
        ps = _resolve_powershell()
        url = os.environ.get("SEASHAIL_INSTALL_URL", "https://seashail.com/install.ps1")
        cmd = f"irm {url} | iex"
        rc = _run(
            [
                ps,
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                cmd,
            ]
        )
        if rc != 0:
            raise RuntimeError(f"installer failed with exit code {rc}")
        return

    url = os.environ.get("SEASHAIL_INSTALL_URL", "https://seashail.com/install")
    cmd = f"curl -fsSL {url} | sh"
    rc = _run(["sh", "-c", cmd])
    if rc != 0:
        raise RuntimeError(f"installer failed with exit code {rc}")


def main() -> None:
    # Support `uvx seashail-mcp -- --network testnet` by passing through args after `--`.
    passthrough = sys.argv[1:]

    # Enable the binary's rate-limited auto-upgrade path by default.
    # Users can disable via SEASHAIL_AUTO_UPGRADE=0 or SEASHAIL_DISABLE_AUTO_UPGRADE=1.
    os.environ.setdefault("SEASHAIL_AUTO_UPGRADE", "1")

    seashail = _resolve_seashail()
    if seashail is None:
        _install_from_source()
        seashail = _resolve_seashail()

    if seashail is None:
        raise RuntimeError("Failed to find the `seashail` binary after install.")

    # Run MCP server over stdio.
    os.execvp(seashail, [seashail, "mcp", *passthrough])


if __name__ == "__main__":
    main()
