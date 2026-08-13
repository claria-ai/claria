#!/usr/bin/env python3
"""Regenerate screenshots and prepare the sibling release website.

Run only after a GitHub release and all of its desktop assets are published.
The script intentionally edits claria.yml (the site's source of truth), never
its generated HTML directly; build.py regenerates the checked-in pages.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request

SCREENSHOTS_DIR = Path(__file__).resolve().parent
REPO_DIR = SCREENSHOTS_DIR.parent


def default_site_dir() -> Path:
    try:
        common_dir = subprocess.run(
            ["git", "rev-parse", "--path-format=absolute", "--git-common-dir"],
            cwd=REPO_DIR,
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()
        primary_checkout = Path(common_dir).resolve().parent
        return primary_checkout.parent / "claria-ai.github.io"
    except (OSError, subprocess.CalledProcessError):
        return REPO_DIR.parent / "claria-ai.github.io"


DEFAULT_SITE_DIR = default_site_dir()
RELEASE_API = "https://api.github.com/repos/claria-ai/claria/releases/tags/v{}"


def configured_release_version(path: Path) -> str:
    in_release = False
    for line in path.read_text().splitlines():
        stripped = line.strip()
        if stripped == "release:":
            in_release = True
            continue
        if in_release and stripped and not line[0].isspace():
            break
        if in_release and stripped.startswith("version:"):
            return stripped.split(":", 1)[1].strip()
    raise RuntimeError(f"no release.version entry found in {path}")


def stable_version_key(version: str) -> tuple[int, int, int]:
    components = version.removeprefix("v").split(".")
    if len(components) != 3 or any(not component.isdecimal() for component in components):
        raise RuntimeError(f"expected a stable X.Y.Z release version, got {version!r}")
    major, minor, patch = (int(component) for component in components)
    return major, minor, patch


def release_assets(version: str) -> dict[str, int]:
    headers = {
        "Accept": "application/vnd.github+json",
        "User-Agent": "claria-release-site",
    }
    if token := os.environ.get("GITHUB_TOKEN"):
        headers["Authorization"] = f"Bearer {token}"
    request = urllib.request.Request(RELEASE_API.format(version), headers=headers)
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            payload = json.load(response)
    except (urllib.error.URLError, json.JSONDecodeError) as error:
        raise RuntimeError(f"could not load GitHub release v{version}: {error}") from error
    return {asset["name"]: int(asset["size"]) for asset in payload.get("assets", [])}


def update_release_config(path: Path, version: str, assets: dict[str, int]) -> None:
    lines = path.read_text().splitlines(keepends=True)
    version_updated = False
    current_suffix: str | None = None
    updated_suffixes: set[str] = set()
    in_release = False

    for index, line in enumerate(lines):
        stripped = line.strip()
        if stripped == "release:":
            in_release = True
            continue
        if in_release and stripped and not line[0].isspace():
            break
        if not in_release:
            continue
        if not version_updated and stripped.startswith("version:"):
            indent = line[: len(line) - len(line.lstrip())]
            ending = "\n" if line.endswith("\n") else ""
            lines[index] = f"{indent}version: {version}{ending}"
            version_updated = True
            continue
        if stripped.startswith("suffix:"):
            current_suffix = stripped.split(":", 1)[1].strip()
            continue
        if stripped.startswith("size:") and current_suffix:
            expected_name = f"Claria_{version}_{current_suffix}"
            if expected_name not in assets:
                raise RuntimeError(
                    f"GitHub release v{version} is missing expected asset {expected_name}"
                )
            indent = line[: len(line) - len(line.lstrip())]
            ending = "\n" if line.endswith("\n") else ""
            megabytes = assets[expected_name] / 1_000_000
            lines[index] = f"{indent}size: {megabytes:.1f} MB{ending}"
            updated_suffixes.add(current_suffix)
            current_suffix = None

    if not version_updated:
        raise RuntimeError(f"no release.version entry found in {path}")
    if not updated_suffixes:
        raise RuntimeError(f"no release artifact entries found in {path}")
    path.write_text("".join(lines))


def copy_site_screenshots(site_dir: Path) -> list[str]:
    output_dir = SCREENSHOTS_DIR / "output"
    image_dir = site_dir / "img"
    copied: list[str] = []
    for destination in sorted(image_dir.glob("*.png")):
        source = output_dir / destination.name
        if source.exists():
            shutil.copy2(source, destination)
            copied.append(destination.name)
    if not copied:
        raise RuntimeError(
            "no generated screenshots matched the website image names; run capture and inspect output/"
        )
    return copied


def run_site_build(site_dir: Path) -> None:
    available = subprocess.run(
        [sys.executable, "-c", "import jinja2, yaml"],
        capture_output=True,
    ).returncode == 0
    if available:
        subprocess.run([sys.executable, "build.py"], cwd=site_dir, check=True)
        return
    if shutil.which("uv"):
        subprocess.run(["uv", "run", "build.py"], cwd=site_dir, check=True)
        return
    with tempfile.TemporaryDirectory(prefix="claria-site-build-") as dependencies:
        subprocess.run(
            [
                sys.executable,
                "-m",
                "pip",
                "install",
                "--quiet",
                "--target",
                dependencies,
                "jinja2",
                "pyyaml",
            ],
            check=True,
        )
        environment = os.environ.copy()
        environment["PYTHONPATH"] = dependencies
        subprocess.run(
            [sys.executable, "build.py"],
            cwd=site_dir,
            env=environment,
            check=True,
        )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("version", help="released version, with or without a leading v")
    parser.add_argument(
        "--site",
        type=Path,
        default=DEFAULT_SITE_DIR,
        help=f"claria-ai.github.io checkout (default: {DEFAULT_SITE_DIR})",
    )
    parser.add_argument(
        "--skip-capture",
        action="store_true",
        help="reuse screenshots/output instead of running Playwright",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    version = args.version.removeprefix("v")
    site_dir = args.site.expanduser().resolve()
    if not (site_dir / "CLAUDE.md").is_file() or not (site_dir / "build.py").is_file():
        raise RuntimeError(f"{site_dir} is not a claria-ai.github.io checkout")

    config_path = site_dir / "claria.yml"
    current_version = configured_release_version(config_path)
    if stable_version_key(current_version) > stable_version_key(version):
        print(
            f"Skipped v{version}: claria-ai.github.io already points at newer "
            f"v{current_version}."
        )
        return 0

    # Fetch release metadata and complete capture before changing the website.
    assets = release_assets(version)
    if not args.skip_capture:
        subprocess.run(["npm", "run", "capture"], cwd=SCREENSHOTS_DIR, check=True)

    update_release_config(config_path, version, assets)
    copied = copy_site_screenshots(site_dir)
    run_site_build(site_dir)
    subprocess.run(["git", "diff", "--check"], cwd=site_dir, check=True)

    print(f"Prepared claria-ai.github.io for v{version}.")
    print(f"Copied {len(copied)} screenshots: {', '.join(copied)}")
    print(f"Review and commit the generated site changes in {site_dir}.")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(1) from error
