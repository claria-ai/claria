from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path

MODULE_PATH = Path(__file__).parents[1] / "update_release_site.py"
SPEC = importlib.util.spec_from_file_location("update_release_site", MODULE_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"could not load {MODULE_PATH}")
update_release_site = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(update_release_site)


class ReleaseSiteConfigTests(unittest.TestCase):
    def test_reads_release_version_without_matching_other_sections(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "claria.yml"
            path.write_text(
                "site:\n"
                "  version: ignored\n"
                "release:\n"
                "  version: 0.23.1\n"
                "other:\n"
                "  version: ignored-too\n"
            )

            self.assertEqual(
                update_release_site.configured_release_version(path),
                "0.23.1",
            )

    def test_stable_versions_compare_numerically(self) -> None:
        self.assertGreater(
            update_release_site.stable_version_key("v0.24.0"),
            update_release_site.stable_version_key("0.23.10"),
        )
        with self.assertRaisesRegex(RuntimeError, "stable X.Y.Z"):
            update_release_site.stable_version_key("0.24.0-rc.1")

    def test_updates_version_and_rounded_asset_sizes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "claria.yml"
            path.write_text(
                "site:\n"
                "  version: unchanged\n"
                "release:\n"
                "  version: 0.23.1\n"
                "  artifacts:\n"
                "    - platform: macOS\n"
                "      suffix: aarch64.dmg\n"
                "      size: 1.0 MB\n"
                "    - platform: Windows\n"
                "      suffix: x64-setup.exe\n"
                "      size: 1.0 MB\n"
            )

            update_release_site.update_release_config(
                path,
                "0.24.0",
                {
                    "Claria_0.24.0_aarch64.dmg": 12_121_077,
                    "Claria_0.24.0_x64-setup.exe": 6_123_760,
                },
            )

            self.assertEqual(
                path.read_text(),
                "site:\n"
                "  version: unchanged\n"
                "release:\n"
                "  version: 0.24.0\n"
                "  artifacts:\n"
                "    - platform: macOS\n"
                "      suffix: aarch64.dmg\n"
                "      size: 12.1 MB\n"
                "    - platform: Windows\n"
                "      suffix: x64-setup.exe\n"
                "      size: 6.1 MB\n",
            )

    def test_missing_asset_does_not_partially_rewrite_config(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "claria.yml"
            original = (
                "release:\n"
                "  version: 0.23.1\n"
                "  artifacts:\n"
                "    - platform: macOS\n"
                "      suffix: aarch64.dmg\n"
                "      size: 12.1 MB\n"
            )
            path.write_text(original)

            with self.assertRaisesRegex(RuntimeError, "missing expected asset"):
                update_release_site.update_release_config(path, "0.24.0", {})

            self.assertEqual(path.read_text(), original)


if __name__ == "__main__":
    unittest.main()
