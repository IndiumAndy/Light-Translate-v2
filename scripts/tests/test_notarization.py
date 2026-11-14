"""Verify resumable Apple submissions and fail-closed network recovery."""
from datetime import datetime, timezone
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
            if arguments[0] == "history":
                return {"history": []}, ""
            return next(iterator)

        def fetch_log(submission_id):
            self.calls.append(["log", submission_id])
            return "statusSummary: withheld"

        return notary.notarize(self.payload, self.receipt, timeout, 30,
                              request=request, now=self.clock.now, sleep=self.clock.sleep,
                              fetch_log=fetch_log)

    def test_transport_errors_resume_one_upload_until_accepted(self):
        result = self.execute([
            ({"id": "saved-id"}, ""),
            ({}, "Internet connection appears to be offline"),
            ({}, "HTTP request timed out"),
            ({"status": "Accepted"}, ""),
        ], timeout=150)
        self.assertEqual(result["status"], "Accepted")
        self.assertEqual([c[0] for c in self.calls], ["history", "submit", "info", "info", "info"])
        self.assertEqual(json.loads(self.receipt.read_text())["id"], "saved-id")

    def test_timeout_can_resume_same_digest_without_resubmitting(self):
        with self.assertRaises(notary.NotarizationError):
            self.execute([
                ({"id": "saved-id"}, ""),
                ({"status": "In Progress"}, ""),
                ({"status": "In Progress"}, ""),
            ], timeout=60)
        self.clock = Clock()
        self.calls = []
        self.execute([({"status": "Accepted"}, "")])
        self.assertEqual(self.calls, [["info", "saved-id"]])

    def test_changed_archive_cannot_reuse_accepted_receipt(self):
        self.receipt.write_text(json.dumps({"id": "wrong-archive", "status": "Accepted", "sha256": "wrong"}))
        self.execute([({"id": "new-id"}, ""), ({"status": "Accepted"}, "")])
        self.assertEqual([c[0] for c in self.calls], ["history", "submit", "info"])
        self.assertEqual(json.loads(self.receipt.read_text())["id"], "new-id")

    def test_invalid_status_stops_with_saved_id(self):
        with self.assertRaisesRegex(notary.NotarizationError, "rejected.*saved-id"):
            self.execute([({"id": "saved-id"}, ""), ({"status": "Invalid"}, "")])
        self.assertEqual([c[0] for c in self.calls], ["history", "submit", "info", "log"])

    def test_error_cannot_promote_an_accepted_response(self):
        with self.assertRaises(notary.NotarizationError):
            self.execute([
                ({"id": "saved-id"}, ""),
                ({"status": "Accepted"}, "incomplete response"),
                ({"status": "Accepted"}, "incomplete response"),
            ], timeout=60)
        self.assertNotEqual(json.loads(self.receipt.read_text())["status"], "Accepted")

    def test_upload_error_with_id_is_resumed_not_uploaded_again(self):
        self.execute([({"id": "saved-id"}, "upload response interrupted"), ({"status": "Accepted"}, "")])
        self.assertEqual([c[0] for c in self.calls], ["history", "submit", "info"])

    def test_response_for_another_submission_is_rejected(self):
        with self.assertRaisesRegex(notary.NotarizationError, "does not match"):
            self.execute([({"id": "saved-id"}, ""), ({"id": "other-id", "status": "Accepted"}, "")])

    def test_archive_mutation_during_processing_is_rejected(self):
        def request(arguments, request_timeout):
            if arguments[0] == "history":
                return {"history": []}, ""
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

    def notarize_with(self, request, wall=None):
        return notary.notarize(
            self.payload, self.receipt, 120, 30, request=request,
            now=self.clock.now, sleep=self.clock.sleep,
            fetch_log=lambda _submission_id: "withheld",
            wall_clock=lambda: wall or datetime(2026, 11, 14, 9, 0, tzinfo=timezone.utc))

    def test_recent_pending_submission_blocks_a_new_upload(self):
        def request(arguments, _timeout):
            self.calls.append(arguments[0])
            return {"history": [{
                "id": "11111111-1111-1111-1111-111111111111",
                "status": "In Progress",
                "createdDate": "2026-11-14T08:00:00.000Z",
            }]}, ""

        with self.assertRaisesRegex(notary.NotarizationError, "11111111-1111-1111-1111-111111111111"):
            self.notarize_with(request)
        self.assertEqual(self.calls, ["history"])

    def test_pending_submission_without_a_timestamp_blocks_upload(self):
        def request(arguments, _timeout):
            self.calls.append(arguments[0])
            return {"history": [{
                "id": "66666666-6666-6666-6666-666666666666",
                "status": "In Progress",
            }]}, ""

        with self.assertRaisesRegex(notary.NotarizationError, "66666666-6666-6666-6666-666666666666"):
            self.notarize_with(request)
        self.assertEqual(self.calls, ["history"])

    def test_submission_from_this_attempt_does_not_block_another_artifact(self):
        def request(arguments, _timeout):
            self.calls.append(arguments[0])
            if arguments[0] == "history":
                return {"history": [{
                    "id": "22222222-2222-2222-2222-222222222222",
                    "status": "In Progress",
                    "createdDate": "2026-11-14T08:45:00.000Z",
                }]}, ""
            if arguments[0] == "submit":
                return {"id": "33333333-3333-3333-3333-333333333333"}, ""
            return {"id": "33333333-3333-3333-3333-333333333333", "status": "Accepted"}, ""

        with patch.dict(os.environ, {"LT_NOTARY_OWN_SUBMISSIONS": "22222222-2222-2222-2222-222222222222"}):
            result = self.notarize_with(request)
        self.assertEqual(result["status"], "Accepted")
        self.assertEqual(self.calls, ["history", "submit", "info"])

    def test_another_runs_newer_submission_still_blocks(self):
        def request(arguments, _timeout):
            self.calls.append(arguments[0])
            return {"history": [{
                "id": "99999999-9999-9999-9999-999999999999",
                "status": "In Progress",
                "createdDate": "2026-11-14T08:45:00.000Z",
            }]}, ""

        with patch.dict(os.environ, {"LT_NOTARY_OWN_SUBMISSIONS": "22222222-2222-2222-2222-222222222222"}):
            with self.assertRaisesRegex(notary.NotarizationError, "99999999-9999-9999-9999-999999999999"):
                self.notarize_with(request)
        self.assertEqual(self.calls, ["history"])

    def test_only_an_accepted_matching_receipt_is_owned(self):
        self.receipt.write_text(json.dumps({
            "id": "22222222-2222-2222-2222-222222222222",
            "status": "Accepted",
            "sha256": notary.archive_hash(self.payload),
        }))
        self.assertEqual(notary.accepted_submission_id(self.payload), "22222222-2222-2222-2222-222222222222")
        self.payload.write_bytes(b"different-bytes")
        self.assertEqual(notary.accepted_submission_id(self.payload), "")
        self.payload.write_bytes(b"signed-installer")
        self.receipt.write_text(json.dumps({
            "id": "22222222-2222-2222-2222-222222222222",
            "status": "Submitted",
            "sha256": notary.archive_hash(self.payload),
        }))
        self.assertEqual(notary.accepted_submission_id(self.payload), "")

    def test_old_pending_submission_does_not_block_a_later_release(self):
        def request(arguments, _timeout):
            self.calls.append(arguments[0])
            if arguments[0] == "history":
                return {"history": [{
                    "id": "77777777-7777-7777-7777-777777777777",
                    "status": "In Progress",
                    "createdDate": "2026-11-13T08:00:00.000Z",
                }]}, ""
            if arguments[0] == "submit":
                return {"id": "88888888-8888-8888-8888-888888888888"}, ""
            return {"id": "88888888-8888-8888-8888-888888888888", "status": "Accepted"}, ""

        result = self.notarize_with(request)
        self.assertEqual(result["status"], "Accepted")
        self.assertEqual(self.calls, ["history", "submit", "info"])

    def test_upload_without_an_id_is_not_retried(self):
        def request(arguments, _timeout):
            self.calls.append(arguments[0])
            if arguments[0] == "history":
                return {"history": []}, ""
            return {}, "connection reset"

        with self.assertRaisesRegex(notary.NotarizationError, "not uploading again"):
            self.notarize_with(request)
        self.assertEqual(self.calls, ["history", "submit"])

    def test_unreadable_history_never_submits(self):
        def request(arguments, _timeout):
            self.calls.append(arguments[0])
            return {}, "offline"

        with self.assertRaisesRegex(notary.NotarizationError, "history"):
            self.notarize_with(request)
        self.assertEqual(self.calls, ["history"])

    def test_query_reads_rejection_log_without_uploading(self):
        def request(arguments, _timeout):
            self.calls.append(arguments[0])
            return {"id": arguments[1], "status": "Invalid"}, ""

        def fetch_log(_submission_id):
            self.calls.append("log")
            return "statusSummary: unsigned executable"

        result = notary.query_submission(
            "44444444-4444-4444-4444-444444444444", max_polls=1,
            request=request, fetch_log=fetch_log, sleep=self.clock.sleep)
        self.assertEqual(result["status"], "Invalid")
        self.assertIn("unsigned executable", result["detail"])
        self.assertEqual(self.calls, ["info", "log"])

    def test_query_in_progress_does_not_fetch_a_log_or_upload(self):
        def request(arguments, _timeout):
            self.calls.append(arguments[0])
            return {"id": arguments[1], "status": "In Progress"}, ""

        def fetch_log(_submission_id):
            raise AssertionError("log")

        result = notary.query_submission(
            "55555555-5555-5555-5555-555555555555", max_polls=1,
            request=request, fetch_log=fetch_log, sleep=self.clock.sleep)
        self.assertEqual(result["status"], "In Progress")
        self.assertEqual(self.calls, ["info"])


if __name__ == "__main__":
    unittest.main()
