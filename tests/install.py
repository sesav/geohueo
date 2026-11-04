"""Offline installer tests. Run with python3 tests/install.py on Linux or macOS."""

import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
import unittest


INSTALLER = Path(__file__).resolve().parents[1] / "install.sh"
REPOSITORY = "https://github.com/sesav/geohueo"
VERSION = "v1.2.3"
BINARY = b"#!/bin/sh\nprintf 'geohueo 1.2.3\\n'\n"

# HTTP and platform detection are mocked. File operations and hashing use real
# tools, except when a test deliberately replaces a command to simulate failure.
MOCK_CURL = r'''
import json
import os
from pathlib import Path
import shutil
import sys

args = sys.argv[1:]
with open(os.environ["INSTALL_TEST_CURL_LOG"], "a") as log:
    log.write(json.dumps(args) + "\n")
url = args[-1]
if url.endswith("/releases/latest"):
    if os.environ.get("INSTALL_TEST_FAIL_LATEST"):
        sys.exit(22)
    print(os.environ["INSTALL_TEST_LATEST"], end="")
else:
    relative = url.split("/releases/download/", 1)[1]
    source = Path(os.environ["INSTALL_TEST_RELEASES"]) / relative
    if not source.is_file():
        sys.exit(22)
    destination = args[args.index("--output") + 1]
    if source.name == os.environ.get("INSTALL_TEST_FAIL_ASSET"):
        Path(destination).write_bytes(source.read_bytes()[:16])
        sys.exit(18)  # curl: partial download
    shutil.copyfile(source, destination)
'''

MOCK_UNAME = '''\
#!/bin/sh
case "$1" in
    -s) printf '%s\\n' "$INSTALL_TEST_OS" ;;
    -m) printf '%s\\n' "$INSTALL_TEST_ARCH" ;;
    *) exit 1 ;;
esac
'''


@unittest.skipIf(os.name == "nt", "install.sh targets Linux and macOS")
class InstallerTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="geohueo-installer-test-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.commands = self.root / "commands"
        self.commands.mkdir()
        for name in ("cat", "grep", "tar", "gzip", "mktemp", "mkdir", "cp", "chmod", "mv", "rm"):
            command = shutil.which(name)
            if command is None:
                self.fail(f"Required test command is missing: {name}")
            (self.commands / name).symlink_to(command)
        for name in ("sha256sum", "shasum"):
            command = shutil.which(name)
            if command:
                (self.commands / name).symlink_to(command)
        self.write_command("curl", f"#!{sys.executable}\n" + MOCK_CURL)
        self.write_command("uname", MOCK_UNAME)
        self.home = self.root / "home with spaces"
        self.home.mkdir()
        self.downloads = self.root / "temporary downloads"
        self.downloads.mkdir()
        self.releases = self.root / "releases"
        self.log = self.root / "curl.jsonl"
        self.destination = self.home / ".local" / "bin" / "geohueo"
        self.env = {
            **os.environ,
            "PATH": str(self.commands),
            "HOME": str(self.home),
            "TMPDIR": str(self.downloads),
            "INSTALL_TEST_RELEASES": str(self.releases),
            "INSTALL_TEST_CURL_LOG": str(self.log),
            "INSTALL_TEST_LATEST": f"{REPOSITORY}/releases/tag/{VERSION}",
            "INSTALL_TEST_OS": "Linux",
            "INSTALL_TEST_ARCH": "x86_64",
        }
        self.archive = self.make_release("x86_64-unknown-linux-musl")

    def write_command(self, name, content):
        path = self.commands / name
        if path.exists() or path.is_symlink():
            path.unlink()
        path.write_text(content)
        path.chmod(0o755)

    def make_release(self, target, version=VERSION, member=None):
        directory = self.releases / version
        directory.mkdir(parents=True, exist_ok=True)
        archive = directory / f"geohueo-{version}-{target}.tar.gz"
        with tarfile.open(archive, "w:gz") as output:
            if member is None:
                member = tarfile.TarInfo("geohueo")
                member.size = len(BINARY)
                member.mode = 0o755
                output.addfile(member, io.BytesIO(BINARY))
            else:
                output.addfile(member)
        self.write_checksum(archive)
        return archive

    def write_checksum(self, archive):
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        Path(f"{archive}.sha256").write_text(f"{digest}  {archive.name}\n")

    def run_installer(self, *args, success=True, piped=False):
        command = ["/bin/sh", str(INSTALLER), *args]
        if piped:
            command = ["/bin/sh", "-s", "--", *args]
        result = subprocess.run(
            command,
            input=INSTALLER.read_text() if piped else None,
            env=self.env,
            cwd=self.home,
            capture_output=True,
            text=True,
            timeout=15,
        )
        if success:
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        else:
            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(list(self.downloads.iterdir()), [], "download directory was not cleaned up")
        self.assertEqual(list(self.home.rglob(".geohueo.*")), [], "staged binary was not cleaned up")
        return result

    def requests(self):
        return [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []

    def existing_installation(self):
        self.destination.parent.mkdir(parents=True)
        self.destination.write_bytes(b"previous installation")

    def test_latest_install_is_verified_executable_and_does_not_edit_profiles(self):
        profile = self.home / ".profile"
        profile.write_text("# leave this alone\n")
        result = self.run_installer()
        self.assertEqual(self.destination.read_bytes(), BINARY)
        self.assertEqual(stat.S_IMODE(self.destination.stat().st_mode), 0o755)
        self.assertEqual(profile.read_text(), "# leave this alone\n")
        self.assertFalse((self.home / ".bashrc").exists())
        self.assertIn("PATH", result.stdout)
        requests = self.requests()
        self.assertEqual(len(requests), 3)
        self.assertIn("--head", requests[0])
        self.assertEqual(requests[0][-1], f"{REPOSITORY}/releases/latest")
        for request in requests:
            for flag in ("--fail", "--location", "--tlsv1.2", "--silent", "--show-error"):
                self.assertIn(flag, request)
            self.assertEqual(request[request.index("--proto") + 1], "=https")
            self.assertEqual(request[request.index("--proto-redir") + 1], "=https")
        for request in requests[1:]:
            self.assertTrue(request[-1].startswith(f"{REPOSITORY}/releases/download/{VERSION}/"))

    def test_explicit_version_custom_directory_and_pipe_to_sh(self):
        destination = self.home / "custom bin"
        self.run_installer("--version", "1.2.3", "--bin-dir", str(destination), piped=True)
        self.assertEqual((destination / "geohueo").read_bytes(), BINARY)
        self.assertFalse(self.destination.exists())
        self.assertEqual(len(self.requests()), 2, "pinned install must not resolve latest")

    def test_existing_binary_can_be_replaced(self):
        self.existing_installation()
        # An in-place overwrite changes this open file too; an atomic rename does not.
        with self.destination.open("rb") as previous:
            self.run_installer("--version", VERSION)
            self.assertEqual(previous.read(), b"previous installation")
        self.assertEqual(self.destination.read_bytes(), BINARY)
        self.assertEqual(stat.S_IMODE(self.destination.stat().st_mode), 0o755)

    def test_platform_mapping(self):
        for os_name, architecture, target in (
            ("Linux", "aarch64", "aarch64-unknown-linux-musl"),
            ("Linux", "arm64", "aarch64-unknown-linux-musl"),
            ("Linux", "amd64", "x86_64-unknown-linux-musl"),
            ("Darwin", "x86_64", "x86_64-apple-darwin"),
            ("Darwin", "arm64", "aarch64-apple-darwin"),
        ):
            with self.subTest(os=os_name, architecture=architecture):
                self.env.update(INSTALL_TEST_OS=os_name, INSTALL_TEST_ARCH=architecture)
                self.make_release(target)
                self.run_installer("--version", VERSION)
                self.assertIn(target, self.requests()[-1][-1])

    def test_prerelease_version(self):
        self.make_release("x86_64-unknown-linux-musl", version="v1.2.3-rc.1")
        self.run_installer("--version", "1.2.3-rc.1")
        self.assertIn("/v1.2.3-rc.1/", self.requests()[0][-1])

    def test_relative_directory(self):
        self.run_installer("--version", VERSION, "--bin-dir", "relative bin")
        self.assertEqual((self.home / "relative bin" / "geohueo").read_bytes(), BINARY)

    def test_custom_directory_without_home(self):
        del self.env["HOME"]
        directory = self.root / "bin"
        self.run_installer("--version", VERSION, "--bin-dir", str(directory))
        self.assertEqual((directory / "geohueo").read_bytes(), BINARY)

    def test_temporary_paths_with_special_characters(self):
        for name in ("temporary\\downloads", "temporary\ndownloads"):
            with self.subTest(name=name):
                self.downloads = self.root / name
                self.downloads.mkdir()
                self.env["TMPDIR"] = str(self.downloads)
                self.run_installer("--version", VERSION)
                self.assertEqual(self.destination.read_bytes(), BINARY)

    def test_checksum_mismatch_preserves_existing_installation(self):
        self.existing_installation()
        with self.archive.open("ab") as output:
            output.write(b"corrupted archive")
        result = self.run_installer("--version", VERSION, success=False)
        self.assertIn("SHA-256 checksum mismatch", result.stderr)
        self.assertEqual(self.destination.read_bytes(), b"previous installation")

    def test_invalid_checksum_does_not_create_installation_directory(self):
        for checksum in ("not a checksum\n", f"{'0' * 64}  wrong-file.tar.gz\n", f"{'z' * 64}  {self.archive.name}\n"):
            with self.subTest(checksum=checksum):
                Path(f"{self.archive}.sha256").write_text(checksum)
                self.run_installer("--version", VERSION, success=False)
                self.assertFalse(self.destination.parent.exists())

    def test_failed_download_preserves_existing_installation(self):
        self.existing_installation()
        for asset in (self.archive.name, self.archive.name + ".sha256"):
            with self.subTest(asset=asset):
                self.env["INSTALL_TEST_FAIL_ASSET"] = asset
                result = self.run_installer("--version", VERSION, success=False)
                self.assertIn("could not download", result.stderr)
                self.assertEqual(self.destination.read_bytes(), b"previous installation")

    def test_failed_staging_preserves_existing_installation(self):
        self.existing_installation()
        self.write_command("cp", '#!/bin/sh\nprintf partial > "$2"\nexit 1\n')
        result = self.run_installer("--version", VERSION, success=False)
        self.assertIn("could not stage", result.stderr)
        self.assertEqual(self.destination.read_bytes(), b"previous installation")

    def test_failed_replacement_preserves_existing_installation(self):
        self.existing_installation()
        self.write_command("mv", "#!/bin/sh\nexit 1\n")
        result = self.run_installer("--version", VERSION, success=False)
        self.assertIn("could not install", result.stderr)
        self.assertEqual(self.destination.read_bytes(), b"previous installation")

    def test_corrupt_archive_is_rejected_even_with_matching_checksum(self):
        self.archive.write_bytes(b"not a gzip archive")
        self.write_checksum(self.archive)
        result = self.run_installer("--version", VERSION, success=False)
        self.assertIn("could not unpack", result.stderr)
        self.assertFalse(self.destination.exists())

    def test_archive_symlink_is_not_installed(self):
        member = tarfile.TarInfo("geohueo")
        member.type = tarfile.SYMTYPE
        member.linkname = "/bin/sh"
        self.make_release("x86_64-unknown-linux-musl", member=member)
        self.run_installer("--version", VERSION, success=False)
        self.assertFalse(self.destination.exists())

    def test_existing_directory_or_symlink_is_not_replaced(self):
        self.destination.mkdir(parents=True)
        self.run_installer("--version", VERSION, success=False)
        self.assertTrue(self.destination.is_dir())
        self.destination.rmdir()
        original = self.home / "original"
        original.write_text("original")
        self.destination.symlink_to(original)
        self.run_installer("--version", VERSION, success=False)
        self.assertTrue(self.destination.is_symlink())
        self.assertEqual(original.read_text(), "original")

    def use_shasum(self):
        if not (self.commands / "shasum").exists():
            self.skipTest("shasum is not installed")
        sha256sum = self.commands / "sha256sum"
        if sha256sum.exists():
            sha256sum.unlink()

    def test_shasum_fallback(self):
        self.use_shasum()
        self.run_installer("--version", VERSION)
        self.assertEqual(self.destination.read_bytes(), BINARY)

    def test_shasum_mismatch_preserves_existing_installation(self):
        self.use_shasum()
        self.existing_installation()
        with self.archive.open("ab") as output:
            output.write(b"corrupted archive")
        result = self.run_installer("--version", VERSION, success=False)
        self.assertIn("SHA-256 checksum mismatch", result.stderr)
        self.assertEqual(self.destination.read_bytes(), b"previous installation")

    def test_missing_checksum_tool_fails_before_download(self):
        for name in ("sha256sum", "shasum"):
            path = self.commands / name
            if path.exists():
                path.unlink()
        result = self.run_installer(success=False)
        self.assertIn("SHA-256 verification requires", result.stderr)
        self.assertEqual(self.requests(), [])

    def test_missing_dependency_fails_before_download(self):
        (self.commands / "tar").unlink()
        result = self.run_installer(success=False)
        self.assertIn("required command not found: tar", result.stderr)
        self.assertEqual(self.requests(), [])

    def test_unsupported_platforms_fail_before_download(self):
        for os_name, architecture in (("FreeBSD", "x86_64"), ("Linux", "i686"), ("MINGW64_NT", "x86_64")):
            with self.subTest(os=os_name, architecture=architecture):
                self.env.update(INSTALL_TEST_OS=os_name, INSTALL_TEST_ARCH=architecture)
                self.run_installer(success=False)
                self.assertEqual(self.requests(), [])

    def test_help_needs_no_network(self):
        result = self.run_installer("--help")
        self.assertIn("Usage:", result.stdout)
        self.assertEqual(self.requests(), [])

    def test_invalid_arguments_and_versions_fail_before_download(self):
        for args in (
            ("--unknown",), ("--version",), ("--bin-dir",), ("--version", ""), ("--bin-dir", ""),
            ("--version", "../v1.2.3"), ("--version", "1.2.3\nbad"), ("--version", "latest"),
        ):
            with self.subTest(args=args):
                self.run_installer(*args, success=False)
                self.assertEqual(self.requests(), [])

    def test_latest_resolution_failure_does_not_install(self):
        self.env["INSTALL_TEST_FAIL_LATEST"] = "1"
        self.run_installer(success=False)
        self.assertFalse(self.destination.exists())
        self.assertEqual(len(self.requests()), 1)

    def test_unexpected_latest_redirect_does_not_install(self):
        self.env["INSTALL_TEST_LATEST"] = "https://example.com/v1.2.3"
        self.run_installer(success=False)
        self.assertFalse(self.destination.exists())
        self.assertEqual(len(self.requests()), 1)


if __name__ == "__main__":
    unittest.main(verbosity=2)
