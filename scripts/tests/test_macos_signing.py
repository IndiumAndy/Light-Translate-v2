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
        if failure == 'upload-lost':
            print('connection reset', file=sys.stderr)
            sys.exit(1)
        print(json.dumps({'id': 'test-request'}))
    elif args[:2] == ['notarytool', 'history']:
        if failure == 'pending-queue':
            print(json.dumps({'history': [{'id': 'pending-request', 'status': 'In Progress'}]}))
        else:
            print(json.dumps({'history': []}))
    elif args[:2] == ['notarytool', 'info']:
        if failure == 'notary-timeout':
            print('Network timeout', file=sys.stderr)
            sys.exit(1)
        print(json.dumps({'status': 'Invalid' if failure == 'notary' else 'Accepted', 'id': 'test-request'}))
    elif args[:2] == ['stapler', 'validate'] and failure == 'ticket':
        sys.exit(1)
elif name == 'spctl' and failure == 'gatekeeper':
    sys.exit(1)
elif name == 'lipo' and failure == 'architecture':
    sys.exit(1)
elif name == 'hdiutil':
    if args[0] == 'create':
        pathlib.Path(args[-1]).touch()
    elif args[0] == 'attach':
        mount = pathlib.Path(args[args.index('-mountpoint') + 1])
        (mount / 'Applications').symlink_to('/Applications')
        if failure != 'missing-app':
            contents = mount / 'Light Translate.app' / 'Contents'
            (contents / 'MacOS').mkdir(parents=True)
            (contents / 'MacOS' / 'light-translate-v2').touch()
            with (contents / 'Info.plist').open('wb') as stream:
                plistlib.dump({'CFBundleIdentifier': 'wrong.bundle' if failure == 'bundle' else 'com.atat.lighttranslate.v2'}, stream)
    elif args[0] == 'detach':
        for child in pathlib.Path(args[1]).iterdir():
            shutil.rmtree(child) if child.is_dir() and not child.is_symlink() else child.unlink()
elif name == 'pnpm':
    if any(os.environ.get(key) for key in ('APPLE_ID', 'APPLE_PASSWORD', 'APPLE_TEAM_ID')):
        sys.exit('Build must not start Tauri notarization')
    app = pathlib.Path(os.environ['CARGO_TARGET_DIR']) / 'universal-apple-darwin/release/bundle/macos/Light Translate.app'
    app.mkdir(parents=True)
elif name == 'ditto':
    if '-c' in args:
        pathlib.Path(args[-1]).write_bytes(b'signed-app-archive')
    else:
        shutil.copytree(args[-2], args[-1])
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
        for command in ("codesign", "xcrun", "spctl", "lipo", "hdiutil", "security", "pnpm", "ditto"):
            (self.bin / command).symlink_to(fake)
        self.env = {k: v for k, v in os.environ.items() if not k.startswith("APPLE_")}
        self.env.update(CREDENTIALS)
        self.env["PATH"] = f"{self.bin}{os.pathsep}{self.env['PATH']}"
        self.log = self.base / "tools.jsonl"
        self.env["TOOL_LOG"] = str(self.log)
        self.env["LT_NOTARY_POLL_SECONDS"] = "0.01"
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

    def test_timeout_preserves_request_id_and_never_staples(self):
        self.env["FAIL_CHECK"] = "notary-timeout"
        self.env["LT_NOTARY_TIMEOUT"] = "1s"
        result = self.run_script("notarize-macos-dmg.sh", self.dmg)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("test-request", result.stderr)
        self.assertFalse(any(c[:2] == ["xcrun", "stapler"] for c in self.calls()))
        submits = [c for c in self.calls() if c[:3] == ["xcrun", "notarytool", "submit"]]
        self.assertEqual(len(submits), 1)
        self.assertNotIn("--wait", submits[0])
        self.assertEqual(json.loads(Path(str(self.dmg) + ".notarization.json").read_text())["id"], "test-request")

    def test_release_packages_the_stapled_app_before_notarizing_dmg(self):
        self.env["CARGO_TARGET_DIR"] = str(self.base / "target")
        result = self.run_script("build-macos-release.sh")
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.calls()
        submitted = [c for c in calls if c[:3] == ["xcrun", "notarytool", "submit"]]
        self.assertEqual(len(submitted), 1)
        self.assertTrue(submitted[0][3].endswith(".zip"))
        staple_index = next(i for i, c in enumerate(calls) if c[:3] == ["xcrun", "stapler", "staple"])
        image_index = next(i for i, c in enumerate(calls) if c[:2] == ["hdiutil", "create"])
        self.assertLess(staple_index, image_index)
        self.assertTrue(any(c[0] == "codesign" and c[-1].endswith(".dmg") for c in calls[image_index:]))

    def test_rejected_app_never_becomes_an_installer(self):
        self.env["CARGO_TARGET_DIR"] = str(self.base / "target")
        self.env["FAIL_CHECK"] = "notary"
        result = self.run_script("build-macos-release.sh")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(any(c[:2] == ["hdiutil", "create"] for c in self.calls()))

    def test_ci_keychain_setup_does_not_run_on_developers_machine(self):
        self.env.pop("GITHUB_ACTIONS", None)
        result = self.run_script("setup-macos-ci-signing.sh")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.calls(), [])

    def test_pending_submission_stops_before_a_new_build(self):
        self.env["FAIL_CHECK"] = "pending-queue"
        self.env["CARGO_TARGET_DIR"] = str(self.base / "target")
        result = self.run_script("build-macos-release.sh")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("pending-request", result.stderr)
        self.assertFalse(any(c[0] == "pnpm" or c[:3] == ["xcrun", "notarytool", "submit"] for c in self.calls()))

    def test_lost_upload_response_is_not_sent_again(self):
        self.env["FAIL_CHECK"] = "upload-lost"
        result = self.run_script("notarize-macos-dmg.sh", self.dmg)
        self.assertNotEqual(result.returncode, 0)
        submits = [c for c in self.calls() if len(c) >= 3 and c[:3] == ["xcrun", "notarytool", "submit"]]
        self.assertEqual(len(submits), 1)

    def test_query_workflow_cannot_start_from_a_push_or_pull_request(self):
        query = (ROOT / ".github/workflows/macos-notary-query.yml").read_text()
        triggers = query.split("on:", 1)[1].split("permissions:", 1)[0]
        self.assertIn("workflow_dispatch:", triggers)
        for forbidden in ("push:", "pull_request:", "schedule:", "workflow_call:"):
            self.assertNotIn(forbidden, triggers)
        self.assertNotIn("notarytool submit", query)
        build = (ROOT / ".github/workflows/build.yml").read_text()
        submit_block = build.split("submit_new:", 1)[1].split("permissions:", 1)[0]
        self.assertIn("default: false", submit_block)
        self.assertIn("inputs.submit_new", build)

    def test_local_installer_does_not_fall_back_to_adhoc(self):
        self.env["APPLE_SIGNING_IDENTITY"] = "-"
        result = self.run_script("install-macos.sh")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.calls(), [])


if __name__ == "__main__":
    unittest.main()
