#!/usr/bin/env python3
"""Submit once, persist the archive identity, and retry transient Apple queries."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time


class NotarizationError(Exception):
    pass


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


def notarize(payload, receipt_path, timeout, poll_interval=30,
             request=run_request, now=time.monotonic, sleep=time.sleep):
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
    previous_status = None
    while now() < deadline:
        if not submission_id:
            data, error = request(["submit", str(payload)], min(300, max(1, deadline - now())))
            # A transport error may still include a valid submission ID. Persist
            # it immediately; do not upload a different archive under that ID.
            candidate_id = data.get("id")
            submission_id = candidate_id if isinstance(candidate_id, str) and candidate_id.strip() else None
            if submission_id:
                receipt.update(id=submission_id, status="Submitted")
                save_receipt(receipt_path, receipt)
                print(f"Apple submission saved: {submission_id}", flush=True)
            elif error:
                print(f"Apple upload will retry: {redact(error)}", file=sys.stderr, flush=True)
            else:
                raise NotarizationError("Apple upload returned no submission ID")
        else:
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
                    print(f"Apple submission {submission_id}: {status}", flush=True)
                    previous_status = status
                if status == "Accepted":
                    if archive_hash(payload) != checksum:
                        raise NotarizationError("Notarization payload changed while Apple was processing it")
                    return receipt
                if status in ("Invalid", "Rejected"):
                    raise NotarizationError(f"Apple notarization rejected: {status} ({submission_id})")
        remaining = deadline - now()
        if remaining > 0:
            sleep(min(poll_interval, remaining))
    raise NotarizationError(f"Apple processing timed out; saved submission ID: {submission_id or 'not uploaded'}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("payload", type=Path)
    parser.add_argument("--receipt", type=Path)
    parser.add_argument("--staple", type=Path)
    args = parser.parse_args()
    try:
        timeout = duration_seconds(os.environ.get("LT_NOTARY_TIMEOUT", "2h"))
        interval = float(os.environ.get("LT_NOTARY_POLL_SECONDS", "30"))
        if interval <= 0:
            raise ValueError("LT_NOTARY_POLL_SECONDS must be positive")
        if not args.payload.is_file():
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
