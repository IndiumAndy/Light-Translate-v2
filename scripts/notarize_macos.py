#!/usr/bin/env python3
"""Submit once, persist the archive identity, and retry transient Apple queries."""
import argparse
from datetime import datetime, timedelta, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import time


class NotarizationError(Exception):
    pass


SUBMISSION_ID = re.compile(
    r"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$")
TERMINAL_FAILURE = {"Invalid", "Rejected"}


def duration_seconds(value):
    match = re.fullmatch(r"(\d+)([smhd]?)", value)
    if not match or int(match[1]) <= 0:
        raise ValueError("LT_NOTARY_TIMEOUT must be a positive duration, such as 2h")
    return int(match[1]) * {"": 1, "s": 1, "m": 60, "h": 3600, "d": 86400}[match[2]]


def redact(value):
    for key in ("APPLE_PASSWORD", "APPLE_ID"):
        secret = os.environ.get(key)
        if secret:
            value = value.replace(secret, "[redacted]")
    return value


def run_request(arguments, request_timeout):
    required = ("APPLE_ID", "APPLE_PASSWORD", "APPLE_TEAM_ID")
    if any(not os.environ.get(key) for key in required):
        raise NotarizationError("Missing Apple notarization credentials")
    command = ["xcrun", "notarytool", *arguments,
               "--apple-id", os.environ["APPLE_ID"],
               "--password", os.environ["APPLE_PASSWORD"],
               "--team-id", os.environ["APPLE_TEAM_ID"],
               "--output-format", "json"]
    try:
        result = subprocess.run(command, capture_output=True, text=True,
                                timeout=request_timeout)
    except subprocess.TimeoutExpired:
        return {}, "Apple request timed out; the saved submission can be queried again"
    try:
        data = json.loads(result.stdout)
    except (json.JSONDecodeError, ValueError):
        data = {}
    if not isinstance(data, dict):
        data = {}
    error = "" if result.returncode == 0 else redact(result.stderr or result.stdout)
    return data, error[:1000]


def archive_hash(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def save_receipt(path, receipt):
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_text(json.dumps(receipt, indent=2) + "\n")
    temporary.replace(path)


def staple(path, run=subprocess.run, sleep=time.sleep):
    for attempt in range(5):
        try:
            result = run(["xcrun", "stapler", "staple", str(path)],
                         capture_output=True, text=True, timeout=120)
            if result.returncode == 0:
                validation = run(["xcrun", "stapler", "validate", str(path)],
                                 capture_output=True, text=True, timeout=120)
                if validation.returncode == 0:
                    print(f"Stapled and validated: {path.name}", flush=True)
                    return
        except subprocess.TimeoutExpired:
            pass
        if attempt < 4:
            print(f"Retrying Apple ticket retrieval for {path.name}", flush=True)
            sleep(15)
    raise NotarizationError(f"Could not staple and validate Apple ticket: {path.name}")


def parse_timestamp(value):
    if not isinstance(value, str) or not value.strip():
        return None
    try:
        parsed = datetime.fromisoformat(value.strip().replace("Z", "+00:00"))
    except ValueError:
        return None
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed.astimezone(timezone.utc)


def submit_after_from_env():
    raw = os.environ.get("LT_NOTARY_SUBMIT_AFTER", "").strip()
    if not raw:
        return None
    parsed = parse_timestamp(raw)
    if parsed is None:
        raise NotarizationError("LT_NOTARY_SUBMIT_AFTER must be a UTC timestamp")
    return parsed


def pending_window():
    raw = os.environ.get("LT_NOTARY_PENDING_HOURS", "12")
    try:
        hours = float(raw)
    except ValueError as error:
        raise NotarizationError("LT_NOTARY_PENDING_HOURS must be a positive number") from error
    if hours <= 0:
        raise NotarizationError("LT_NOTARY_PENDING_HOURS must be a positive number")
    return timedelta(hours=hours)


def history_entries(data, error):
    if error:
        raise NotarizationError(
            "Refusing to upload because Apple history could not be read: " + redact(error))
    if not data:
        return []
    history = data.get("history")
    if not isinstance(history, list):
        raise NotarizationError(
            "Apple history response has no submission list; refusing to upload")
    return history


def blocking_submission_ids(entries, submit_after, wall_time):
    window_start = wall_time - pending_window()
    blocked = []
    for entry in entries:
        if not isinstance(entry, dict) or entry.get("status") != "In Progress":
            continue
        submission_id = entry.get("id")
        if not isinstance(submission_id, str) or not submission_id.strip():
            blocked.append("unknown")
            continue
        submission_id = submission_id.strip()
        created = parse_timestamp(entry.get("createdDate"))
        if created is not None and created < window_start:
            continue
        if submit_after is not None and created is not None and created >= submit_after:
            continue
        blocked.append(submission_id)
    return blocked


def refuse_pending_uploads(request, request_timeout, wall_clock=None):
    wall_clock = wall_clock or (lambda: datetime.now(timezone.utc))
    data, error = request(["history"], request_timeout)
    blocked = blocking_submission_ids(
        history_entries(data, error), submit_after_from_env(), wall_clock())
    if blocked:
        joined = ", ".join(blocked)
        raise NotarizationError(
            "Refusing to upload while Apple still has pending submissions: "
            f"{joined}. Query these IDs; do not rebuild or submit again.")
    return data


def summarize_log(text):
    try:
        data = json.loads(text)
    except (json.JSONDecodeError, TypeError):
        return redact(text)[:2000]
    if not isinstance(data, dict):
        return redact(text)[:2000]
    lines = [str(data.get("statusSummary") or data.get("status") or "Apple validation log")]
    issues = data.get("issues") if isinstance(data.get("issues"), list) else []
    for issue in issues[:20]:
        if not isinstance(issue, dict):
            continue
        lines.append(
            f"{issue.get('severity') or 'issue'}: {issue.get('message') or ''} "
            f"({issue.get('path') or ''})")
    return redact("\n".join(line for line in lines if line.strip()))[:2000]


def fetch_log_text(submission_id):
    if not SUBMISSION_ID.fullmatch(submission_id):
        raise NotarizationError("Not an Apple submission ID")
    destination_dir = Path(tempfile.mkdtemp())
    destination = destination_dir / "notary-log.json"
    try:
        _data, error = run_request(["log", submission_id, str(destination)], 120)
        if destination.is_file() and destination.stat().st_size:
            return summarize_log(destination.read_text(errors="replace"))
        return "Apple log unavailable: " + (error or "empty log")
    finally:
        destination.unlink(missing_ok=True)
        destination_dir.rmdir()


def rejection_error(status, submission_id, fetch_log):
    try:
        detail = fetch_log(submission_id)
    except (NotarizationError, OSError, subprocess.TimeoutExpired) as error:
        detail = f"Apple log unavailable: {error}"
    return NotarizationError(
        f"Apple notarization rejected: {status} ({submission_id}). {redact(str(detail))}")


def notarize(payload, receipt_path, timeout, poll_interval=30,
             request=run_request, now=time.monotonic, sleep=time.sleep,
             fetch_log=fetch_log_text, wall_clock=None):
    checksum = archive_hash(payload)
    receipt = {}
    if receipt_path.exists():
        try:
            receipt = json.loads(receipt_path.read_text())
        except (ValueError, OSError):
            receipt = {}
    if not isinstance(receipt, dict) or receipt.get("sha256") != checksum:
        receipt = {"sha256": checksum, "file": payload.name}
    deadline = now() + timeout
    submission_id = receipt.get("id")
    if not isinstance(submission_id, str) or not submission_id.strip():
        submission_id = None
    if not submission_id:
        refuse_pending_uploads(request, min(120, max(1, deadline - now())), wall_clock)
        # One upload. A lost response must not send the same archive again.
        data, error = request(["submit", str(payload)], min(300, max(1, deadline - now())))
        candidate_id = data.get("id")
        submission_id = candidate_id if isinstance(candidate_id, str) and candidate_id.strip() else None
        if not submission_id:
            raise NotarizationError(
                "Apple upload returned no submission ID; not uploading again. " + redact(error))
        receipt.update(id=submission_id, status="Submitted")
        save_receipt(receipt_path, receipt)
        print(f"Apple submission saved: {submission_id}", flush=True)
    previous_status = None
    while now() < deadline:
        data, error = request(["info", submission_id], min(120, max(1, deadline - now())))
        if error:
            print(f"Apple query will retry for {submission_id}: {redact(error)}",
                  file=sys.stderr, flush=True)
        else:
            if data.get("id") and data["id"] != submission_id:
                raise NotarizationError("Apple response does not match the saved submission ID")
            status = data.get("status")
            receipt.update(status=status)
            save_receipt(receipt_path, receipt)
            if status != previous_status:
                print(f"NOTARY_STATUS id={submission_id} status={status}", flush=True)
                previous_status = status
            if status == "Accepted":
                if archive_hash(payload) != checksum:
                    raise NotarizationError("Notarization payload changed while Apple was processing it")
                return receipt
            if status in TERMINAL_FAILURE:
                raise rejection_error(status, submission_id, fetch_log)
        remaining = deadline - now()
        if remaining <= 0:
            break
        sleep(min(poll_interval, remaining))
    raise NotarizationError(
        f"Apple processing timed out; saved submission ID: {submission_id or 'not uploaded'}. "
        "Query this ID; do not rebuild or submit again.")


def query_submission(submission_id, max_polls=1, poll_interval=30, request=run_request,
                     sleep=time.sleep, fetch_log=fetch_log_text):
    if not SUBMISSION_ID.fullmatch(submission_id):
        raise NotarizationError("Not an Apple submission ID")
    last_error = "Apple returned no status"
    status = None
    for poll in range(max_polls):
        data, error = request(["info", submission_id], 120)
        if error or not isinstance(data, dict) or not data.get("status"):
            last_error = error or "Apple returned no status"
            if poll + 1 < max_polls:
                sleep(poll_interval)
            continue
        if data.get("id") and data["id"] != submission_id:
            raise NotarizationError("Apple response does not match the saved submission ID")
        status = data["status"]
        print(f"NOTARY_STATUS id={submission_id} status={status}", flush=True)
        break
    if status is None:
        raise NotarizationError(
            f"Could not read Apple submission {submission_id}: {redact(last_error)}")
    detail = ""
    if status in TERMINAL_FAILURE:
        detail = redact(str(rejection_error(status, submission_id, fetch_log)))
        print(detail, flush=True)
    return {"id": submission_id, "status": status, "detail": detail}


def query_ids_from_env():
    raw = os.environ.get("NOTARY_QUERY_IDS", "")
    identifiers = [item.strip() for item in raw.split(",") if item.strip()]
    for identifier in identifiers:
        if not SUBMISSION_ID.fullmatch(identifier):
            raise NotarizationError(f"Not an Apple submission ID: {identifier}")
    return identifiers


def write_summary(lines):
    path = os.environ.get("GITHUB_STEP_SUMMARY")
    if not path:
        return
    with open(path, "a", encoding="utf-8") as stream:
        stream.write("\n".join(lines) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("payload", nargs="?", type=Path)
    parser.add_argument("--receipt", type=Path)
    parser.add_argument("--staple", type=Path)
    parser.add_argument("--require-idle", action="store_true")
    parser.add_argument("--query-only", action="store_true")
    args = parser.parse_args()
    try:
        if args.require_idle and args.query_only:
            raise NotarizationError("Choose either --require-idle or --query-only")
        if args.require_idle:
            refuse_pending_uploads(run_request, 120)
            print("NOTARY_STATUS action=idle", flush=True)
            return 0
        if args.query_only:
            polls = int(os.environ.get("LT_NOTARY_MAX_POLLS", "1"))
            if polls < 1:
                raise NotarizationError("LT_NOTARY_MAX_POLLS must be at least 1")
            interval = float(os.environ.get("LT_NOTARY_POLL_SECONDS", "30"))
            results = []
            identifiers = query_ids_from_env()
            history, history_error = run_request(["history"], 120)
            if history_error:
                print("Apple history could not be read: " + redact(history_error),
                      file=sys.stderr, flush=True)
            else:
                try:
                    entries = history_entries(history, "")
                except NotarizationError as error:
                    entries = []
                    history_error = str(error)
                    print(redact(history_error), file=sys.stderr, flush=True)
                for entry in entries:
                    if isinstance(entry, dict) and entry.get("id"):
                        print(
                            "NOTARY_HISTORY "
                            f"id={entry.get('id')} status={entry.get('status')} "
                            f"created={entry.get('createdDate') or ''}",
                            flush=True)
            failed = bool(history_error) and not identifiers
            for submission_id in identifiers:
                try:
                    result = query_submission(submission_id, polls, interval)
                except NotarizationError as error:
                    print(redact(str(error)), file=sys.stderr, flush=True)
                    results.append({"id": submission_id, "status": "unreadable"})
                    failed = True
                    continue
                results.append(result)
                if result["status"] in TERMINAL_FAILURE:
                    failed = True
            write_summary(["# Apple notarization status", ""] + [
                f"- `{result['id']}`: {result['status']}" for result in results
            ])
            return 1 if failed else 0
        timeout = duration_seconds(os.environ.get("LT_NOTARY_TIMEOUT", "2h"))
        interval = float(os.environ.get("LT_NOTARY_POLL_SECONDS", "30"))
        if interval <= 0:
            raise ValueError("LT_NOTARY_POLL_SECONDS must be positive")
        if args.payload is None or not args.payload.is_file():
            raise NotarizationError("Notarization payload does not exist")
        receipt = args.receipt or Path(str(args.payload) + ".notarization.json")
        notarize(args.payload, receipt, timeout, interval)
        if args.staple:
            staple(args.staple)
    except (NotarizationError, ValueError, OSError) as error:
        print(redact(str(error)), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
