"""Exercise release failure boundaries without credentials or Apple services."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SCRIPTS = ROOT / "scripts"
CREDENTIALS = {
    "APPLE_SIGNING_IDENTITY": "Developer ID Application: Test (ABCDE12345)",
    "APPLE_TEAM_ID": "ABCDE12345",
    "APPLE_ID": "test@example.invalid",
    "APPLE_PASSWORD": "test-only",
    "APPLE_CERTIFICATE": "test-only",
    "APPLE_CERTIFICATE_PASSWORD": "test-only",
}
FAKE_TOOL = r'''#!/usr/bin/env python3
import json, os, pathlib, plistlib, shutil, sys
name = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
failure = os.environ.get('FAIL_CHECK', '')
with open(os.environ['TOOL_LOG'], 'a') as stream:
    stream.write(json.dumps([name] + args) + '\n')
if name == 'codesign':
    if '--verify' in args:
        if failure == 'signature' or (failure == 'certificate' and any(a.startswith('-R=') for a in args)):
            sys.exit(1)
    elif '--requirements' in args:
        print('designated => cdhash H"1234"' if failure == 'cdhash' else
              'designated => identifier "com.atat.lighttranslate.v2" and anchor apple generic and certificate leaf[subject.OU] = "ABCDE12345"')
    else:
        print('TeamIdentifier=' + ('WRONG12345' if failure == 'team' else 'ABCDE12345'))
        print('Authority=' + ('Apple Development: Test' if failure == 'development' else 'Developer ID Application: Test'))
        print('CodeDirectory v=20500 flags=' + ('0x0(none)' if failure == 'runtime' else '0x10000(runtime)'))
        if failure != 'timestamp':
            print('Timestamp=Nov 13, 2026')
elif name == 'xcrun':
    if args[:2] == ['notarytool', 'submit']:
        print(json.dumps({'status': 'Invalid' if failure == 'notary' else 'Accepted', 'id': 'test-request'}))
    elif args[:2] == ['stapler', 'validate'] and failure == 'ticket':
        sys.exit(1)
elif name == 'spctl' and failure == 'gatekeeper':
    sys.exit(1)
elif name == 'lipo' and failure == 'architecture':
    sys.exit(1)
elif name == 'hdiutil':
    if args[0] == 'attach':
        mount = pathlib.Path(args[args.index('-mountpoint') + 1])
        if failure != 'missing-app':
            contents = mount / 'Light Translate.app' / 'Contents'
            (contents / 'MacOS').mkdir(parents=True)
            (contents / 'MacOS' / 'light-translate-v2').touch()
            with (contents / 'Info.plist').open('wb') as stream:
                plistlib.dump({'CFBundleIdentifier': 'wrong.bundle' if failure == 'bundle' else 'com.atat.lighttranslate.v2'}, stream)
    elif args[0] == 'detach':
        for child in pathlib.Path(args[1]).iterdir():
            shutil.rmtree(child) if child.is_dir() else child.unlink()
'''


class SigningSafeguards(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.bin = self.base / "bin"
        self.bin.mkdir()
        fake = self.bin / "fake-tool"
        fake.write_text(FAKE_TOOL)
        fake.chmod(0o755)
        for command in ("codesign", "xcrun", "spctl", "lipo", "hdiutil", "security", "pnpm"):
            (self.bin / command).symlink_to(fake)
        self.env = {k: v for k, v in os.environ.items() if not k.startswith("APPLE_")}
        self.env.update(CREDENTIALS)
        self.env["PATH"] = f"{self.bin}{os.pathsep}{self.env['PATH']}"
        self.log = self.base / "tools.jsonl"
        self.env["TOOL_LOG"] = str(self.log)
        self.dmg = self.base / "Light Translate.dmg"
        self.dmg.touch()

    def run_script(self, name, *args):
        return subprocess.run(["bash", str(SCRIPTS / name), *map(str, args)],
                              env=self.env, text=True, capture_output=True)

    def calls(self):
        return [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []

    def test_complete_release_credentials(self):
        result = self.run_script("check-macos-signing-env.sh", "--ci")
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_each_missing_secret_blocks_release_without_printing_values(self):
        for key in CREDENTIALS:
            with self.subTest(secret=key):
                saved = self.env.pop(key)
                result = self.run_script("check-macos-signing-env.sh", "--ci")
                self.env[key] = saved
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(key, result.stderr)
                self.assertNotIn("test-only", result.stderr)

    def test_development_adhoc_and_mismatched_team_block_release(self):
        for identity in ("-", "Apple Development: Test (ABCDE12345)",
                         "Developer ID Application: Test (WRONG12345)"):
            with self.subTest(identity=identity):
                self.env["APPLE_SIGNING_IDENTITY"] = identity
                self.assertNotEqual(self.run_script("check-macos-signing-env.sh").returncode, 0)

    def test_all_release_checks_accept_valid_fixture(self):
        result = self.run_script("verify-macos-release.sh", self.dmg, "ABCDE12345")
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.calls()
        self.assertTrue(any(c[:3] == ["spctl", "--assess", "--type"] and "execute" in c for c in calls))
        self.assertTrue(any(c[:2] == ["hdiutil", "detach"] for c in calls))

    def test_every_failed_apple_check_blocks_release(self):
        for check in ("signature", "certificate", "team", "development", "cdhash", "runtime",
                      "timestamp", "ticket", "gatekeeper", "architecture", "missing-app", "bundle"):
            with self.subTest(check=check):
                self.env["FAIL_CHECK"] = check
                result = self.run_script("verify-macos-release.sh", self.dmg, "ABCDE12345")
                self.assertNotEqual(result.returncode, 0, f"Accepted failed check: {check}")

    def test_rejected_notarization_never_staples_or_verifies(self):
        self.env["FAIL_CHECK"] = "notary"
        result = self.run_script("notarize-macos-dmg.sh", self.dmg)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(any(c[:2] == ["xcrun", "stapler"] for c in self.calls()))

    def test_accepted_notarization_still_requires_gatekeeper(self):
        self.env["FAIL_CHECK"] = "gatekeeper"
        result = self.run_script("notarize-macos-dmg.sh", self.dmg)
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue(any(c[:3] == ["xcrun", "stapler", "staple"] for c in self.calls()))

    def test_ci_keychain_setup_does_not_run_on_developers_machine(self):
        self.env.pop("GITHUB_ACTIONS", None)
        result = self.run_script("setup-macos-ci-signing.sh")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.calls(), [])

    def test_local_installer_does_not_fall_back_to_adhoc(self):
        self.env["APPLE_SIGNING_IDENTITY"] = "-"
        result = self.run_script("install-macos.sh")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.calls(), [])


if __name__ == "__main__":
    unittest.main()
