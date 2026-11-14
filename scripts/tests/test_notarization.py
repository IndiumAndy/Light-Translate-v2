"""Verify resumable Apple submissions and fail-closed network recovery."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

MODULE_PATH = Path(__file__).resolve().parents[1] / "notarize_macos.py"
SPEC = importlib.util.spec_from_file_location("notarize_macos", MODULE_PATH)
notary = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(notary)


class Clock:
    def __init__(self):
        self.elapsed = 0

    def now(self):
        return self.elapsed

    def sleep(self, seconds):
        self.elapsed += seconds


class NotarizationRecovery(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.payload = Path(self.temp.name) / "installer.dmg"
        self.payload.write_bytes(b"signed-installer")
        self.receipt = Path(str(self.payload) + ".notarization.json")
        self.clock = Clock()
        self.calls = []

    def execute(self, responses, timeout=120):
        iterator = iter(responses)

        def request(arguments, request_timeout):
            self.calls.append(arguments)
            self.assertLessEqual(request_timeout, timeout)
            return next(iterator)

        return notary.notarize(self.payload, self.receipt, timeout, 30,
                              request=request, now=self.clock.now, sleep=self.clock.sleep)

    def test_transport_errors_resume_one_upload_until_accepted(self):
        result = self.execute([
            ({"id": "saved-id"}, ""),
            ({}, "Internet connection appears to be offline"),
            ({}, "HTTP request timed out"),
            ({"status": "Accepted"}, ""),
        ], timeout=150)
        self.assertEqual(result["status"], "Accepted")
        self.assertEqual([c[0] for c in self.calls], ["submit", "info", "info", "info"])
        self.assertEqual(json.loads(self.receipt.read_text())["id"], "saved-id")

    def test_timeout_can_resume_same_digest_without_resubmitting(self):
        with self.assertRaises(notary.NotarizationError):
            self.execute([({"id": "saved-id"}, ""), ({"status": "In Progress"}, "")], timeout=60)
        self.clock = Clock()
        self.calls = []
        self.execute([({"status": "Accepted"}, "")])
        self.assertEqual(self.calls, [["info", "saved-id"]])

    def test_changed_archive_cannot_reuse_accepted_receipt(self):
        self.receipt.write_text(json.dumps({"id": "wrong-archive", "status": "Accepted", "sha256": "wrong"}))
        self.execute([({"id": "new-id"}, ""), ({"status": "Accepted"}, "")])
        self.assertEqual(self.calls[0][0], "submit")
        self.assertEqual(json.loads(self.receipt.read_text())["id"], "new-id")

    def test_invalid_status_stops_with_saved_id(self):
        with self.assertRaisesRegex(notary.NotarizationError, "rejected.*saved-id"):
            self.execute([({"id": "saved-id"}, ""), ({"status": "Invalid"}, "")])

    def test_error_cannot_promote_an_accepted_response(self):
        with self.assertRaises(notary.NotarizationError):
            self.execute([({"id": "saved-id"}, ""), ({"status": "Accepted"}, "incomplete response")], timeout=60)
        self.assertNotEqual(json.loads(self.receipt.read_text())["status"], "Accepted")

    def test_upload_error_with_id_is_resumed_not_uploaded_again(self):
        self.execute([({"id": "saved-id"}, "upload response interrupted"), ({"status": "Accepted"}, "")])
        self.assertEqual([c[0] for c in self.calls], ["submit", "info"])

    def test_response_for_another_submission_is_rejected(self):
        with self.assertRaisesRegex(notary.NotarizationError, "does not match"):
            self.execute([({"id": "saved-id"}, ""), ({"id": "other-id", "status": "Accepted"}, "")])

    def test_archive_mutation_during_processing_is_rejected(self):
        def request(arguments, request_timeout):
            if arguments[0] == "submit":
                return {"id": "saved-id"}, ""
            self.payload.write_bytes(b"different-signed-installer")
            return {"status": "Accepted"}, ""

        with self.assertRaisesRegex(notary.NotarizationError, "payload changed"):
            notary.notarize(self.payload, self.receipt, 120, 30, request=request,
                           now=self.clock.now, sleep=self.clock.sleep)

    def test_request_timeout_never_prints_command_credentials(self):
        credentials = {"APPLE_ID": "test@example.invalid", "APPLE_PASSWORD": "secret-test-password", "APPLE_TEAM_ID": "ABCDE12345"}
        with patch.dict(os.environ, credentials), patch.object(notary.subprocess, "run", side_effect=subprocess.TimeoutExpired("sensitive command", 120)):
            data, error = notary.run_request(["info", "saved-id"], 120)
        self.assertEqual(data, {})
        self.assertNotIn("secret-test-password", error)
        self.assertIn("timed out", error)

    def test_duration_supports_long_queues_and_rejects_invalid_input(self):
        self.assertEqual(notary.duration_seconds("2h"), 7200)
        for invalid in ("0s", "-1m", "forever"):
            with self.assertRaises(ValueError):
                notary.duration_seconds(invalid)

    def test_ticket_transport_error_retries_without_resubmission(self):
        calls = []
        responses = iter([subprocess.TimeoutExpired("stapler", 120), 0, 0])

        def run(arguments, **options):
            calls.append(arguments)
            result = next(responses)
            if isinstance(result, Exception):
                raise result
            return subprocess.CompletedProcess(arguments, result, "", "")

        notary.staple(self.payload, run=run, sleep=self.clock.sleep)
        self.assertEqual([c[2] for c in calls], ["staple", "staple", "validate"])

    def test_invalid_ticket_never_finishes_successfully(self):
        def run(arguments, **options):
            return subprocess.CompletedProcess(arguments, 1, "", "")

        with self.assertRaises(notary.NotarizationError):
            notary.staple(self.payload, run=run, sleep=self.clock.sleep)


if __name__ == "__main__":
    unittest.main()
