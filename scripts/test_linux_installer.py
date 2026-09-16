"""Exercise the tarball's actual installer without building Rust or using a real home."""

import os
from pathlib import Path
import subprocess
import tempfile
import unittest


class LinuxInstallerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="supermd-installer-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.payload = self.root / "tarball"
        self.payload.mkdir()
        self.destination = self.root / "installation"
        source = Path(__file__).with_name("bundle_linux.sh").read_text()
        installer = source.split("<<'INSTALL'\n", 1)[1].split("\nINSTALL\n", 1)[0]
        # Remap only the installer destination; do not change the process HOME.
        installer = installer.replace("$HOME", str(self.destination))
        (self.payload / "install.sh").write_text(installer)
        for name in ("supermd", "supermd.desktop", "supermd-128.png", "supermd-512.png"):
            (self.payload / name).write_bytes(b"installer fixture\n")
        # Desktop cache generation is unrelated to installation layout.
        self.tools = self.root / "tools"
        self.tools.mkdir()
        cache_tool = self.tools / "update-desktop-database"
        cache_tool.write_text("#!/bin/sh\nexit 0\n")
        cache_tool.chmod(0o755)

    def install(self):
        subprocess.run(
            ["sh", str(self.payload / "install.sh")],
            env={**os.environ, "PATH": f"{self.tools}:{os.environ['PATH']}"},
            check=True,
            capture_output=True,
            text=True,
        )

    def test_installs_plugins_at_runtime_probe_path_and_updates_in_place(self):
        plugin = self.payload / "plugins" / "word-count"
        plugin.mkdir(parents=True)
        (plugin / "plugin.toml").write_text('name = "word-count"\n')
        (plugin / "plugin.wasm").write_bytes(b"first version")
        user_plugin = self.destination / ".supermd/plugins/custom/plugin.wasm"
        user_plugin.parent.mkdir(parents=True)
        user_plugin.write_bytes(b"user-owned")

        self.install()
        binary = self.destination / ".local/bin/supermd"
        installed = binary.parent / "../lib/supermd/plugins/word-count"
        self.assertTrue(binary.is_file())
        self.assertTrue((installed / "plugin.wasm").is_file(), "bundled plugin must be discoverable beside the installed binary")
        self.assertEqual((installed / "plugin.wasm").read_bytes(), b"first version")
        self.assertEqual((installed / "plugin.toml").read_bytes(), (plugin / "plugin.toml").read_bytes())

        (plugin / "plugin.wasm").write_bytes(b"updated version")
        self.install()
        self.assertEqual((installed / "plugin.wasm").read_bytes(), b"updated version")
        self.assertFalse((installed.parent / "plugins").exists())
        self.assertEqual(user_plugin.read_bytes(), b"user-owned")

    def test_archive_without_optional_plugin_payload_still_installs(self):
        self.install()
        self.assertTrue((self.destination / ".local/bin/supermd").is_file())


if __name__ == "__main__":
    unittest.main()
