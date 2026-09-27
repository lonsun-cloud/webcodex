#!/usr/bin/env python3
"""Optional PostToolUse adapter. Never installs/trusts Hooks or executes work."""
import argparse
import contextlib
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

MAX_BYTES = 131072
MAX_PENDING = 256


class AdapterError(Exception):
    pass


def digest(value):
    return hashlib.sha256(value.encode()).hexdigest()


def private_file(path):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(fd, "rb") as stream:
        st = os.fstat(stream.fileno())
        if not stat.S_ISREG(st.st_mode) or st.st_uid != os.getuid() or st.st_mode & 0o077 or st.st_nlink != 1:
            raise AdapterError("private_regular_file_required")
        raw = stream.read(MAX_BYTES + 1)
        if len(raw) > MAX_BYTES:
            raise AdapterError("input_too_large")
        return raw


def load_config(path):
    config = json.loads(private_file(path))
    required = {"server_url", "authorization_file", "project", "project_root", "workflow_session_id", "local_session_id", "state_dir"}
    if not isinstance(config, dict) or set(config) != required or not all(isinstance(v, str) and v for v in config.values()):
        raise AdapterError("invalid_config")
    root = Path(config["project_root"])
    if not root.is_absolute() or root.resolve() != root or not root.is_dir():
        raise AdapterError("canonical_project_root_required")
    for value in (path, config["authorization_file"], config["state_dir"]):
        p = Path(value)
        if not p.is_absolute() or p.resolve().is_relative_to(root):
            raise AdapterError("configuration_and_state_must_be_outside_project")
    url = urllib.parse.urlsplit(config["server_url"])
    if (url.username or url.password or url.query or url.fragment or url.path not in ("", "/")
            or not url.hostname or (url.scheme != "https" and not
                (url.scheme == "http" and url.hostname in ("127.0.0.1", "::1", "localhost")))):
        raise AdapterError("https_or_loopback_origin_required")
    return config


def observation(config, payload):
    if not isinstance(payload, dict):
        raise AdapterError("invalid_hook_payload")
    if payload.get("hook_event_name") != "PostToolUse":
        return None
    # Trusted configuration chooses identity, not project-controlled Hook input.
    if payload.get("session_id") != config["local_session_id"]:
        raise AdapterError("local_session_mismatch")
    cwd = payload.get("cwd")
    if not isinstance(cwd, str) or not Path(cwd).is_absolute() or Path(cwd).resolve() != Path(config["project_root"]):
        raise AdapterError("project_root_mismatch")
    call_id, tool = payload.get("tool_use_id"), payload.get("tool_name")
    if not isinstance(call_id, str) or not 1 <= len(call_id) <= 256:
        raise AdapterError("missing_tool_identity")
    if not isinstance(tool, str) or not re.fullmatch(r"[A-Za-z0-9_.:-]{1,64}", tool):
        raise AdapterError("invalid_tool_name")
    return {
        "project": config["project"], "session_id": config["workflow_session_id"],
        "adapter_id": digest("codex-session\0" + config["local_session_id"]),
        "event_id": digest("codex-tool\0" + config["local_session_id"] + "\0" + call_id),
        "observed_tool": tool,
        # Generic PostToolUse text is not a terminal execution receipt. This
        # first adapter intentionally reports unknown even if text says success.
        "exit_code": None,
    }


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


def send(config, event, timeout):
    authorization = private_file(config["authorization_file"]).decode().strip()
    if not authorization or "\n" in authorization or "\r" in authorization:
        raise AdapterError("invalid_authorization_file")
    request = urllib.request.Request(config["server_url"].rstrip("/") + "/api/tools/call",
        data=json.dumps({"tool": "record_external_observation", "params": event}).encode(),
        headers={"Content-Type": "application/json", "Authorization": authorization}, method="POST")
    # Never follow redirects with credentials; ambient proxy discovery is not
    # used by this explicitly configured adapter.
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
    with opener.open(request, timeout=timeout) as response:
        raw = response.read(MAX_BYTES + 1)
    if len(raw) > MAX_BYTES:
        raise AdapterError("oversized_response")
    result = json.loads(raw)
    if not isinstance(result, dict) or not isinstance(result.get("output"), dict):
        raise AdapterError("invalid_recording_response")
    output = result["output"]
    received = output.get("observation", {})
    if not isinstance(received, dict):
        raise AdapterError("invalid_recording_response")
    if (result.get("success") is not True or output.get("project") != event["project"]
            or output.get("session_id") != event["session_id"]
            or output.get("provenance") != "external_report"
            or received.get("tool") != event["observed_tool"]
            or any(received.get(key) != event[key] for key in ("adapter_id", "event_id", "exit_code"))):
        raise AdapterError("recording_not_acknowledged")


@contextlib.contextmanager
def state_lock(config):
    root = Path(config["state_dir"])
    # State must be prepared explicitly by the operator; no project file is
    # automatically executable/configuration, and no trust or permissions change.
    fd = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    lock = None
    try:
        st = os.fstat(fd)
        if st.st_uid != os.getuid() or st.st_mode & 0o077:
            raise AdapterError("private_state_directory_required")
        lock = os.open(".lock", os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600, dir_fd=fd)
        ls = os.fstat(lock)
        if not stat.S_ISREG(ls.st_mode) or ls.st_uid != os.getuid() or ls.st_nlink != 1 or ls.st_mode & 0o077:
            raise AdapterError("invalid_lock")
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise AdapterError("adapter_busy_event_not_saved") from None
        yield fd
    finally:
        if lock is not None:
            os.close(lock)
        os.close(fd)


def read_pending(fd, name):
    child = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=fd)
    with os.fdopen(child, "rb") as stream:
        st = os.fstat(stream.fileno())
        if not stat.S_ISREG(st.st_mode) or st.st_nlink != 1 or st.st_uid != os.getuid() or st.st_mode & 0o077:
            raise AdapterError("invalid_pending_file")
        raw = stream.read(MAX_BYTES + 1)
    if len(raw) > MAX_BYTES:
        raise AdapterError("oversized_pending_file")
    return json.loads(raw)


def deliver(config, event=None, sender=send):
    # Never automatically retarget pending events after configuration changes.
    association = digest(json.dumps([config[k] for k in ("server_url", "project", "project_root", "workflow_session_id", "local_session_id")]))
    with state_lock(config) as fd:
        names = sorted(n for n in os.listdir(fd) if re.fullmatch(r"[0-9a-f]{64}\.json", n))
        if event is not None:
            name = digest(association + event["event_id"]) + ".json"
            envelope = {"association": association, "event": event}
            if name in names:
                if read_pending(fd, name) != envelope:
                    raise AdapterError("pending_event_conflict")
            else:
                if len(names) >= MAX_PENDING:
                    raise AdapterError("pending_capacity_event_not_saved")
                # Write + fsync before transmission. An interrupted write is
                # left visible and fails closed; it is never silently replayed.
                child = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=fd)
                with os.fdopen(child, "w") as stream:
                    json.dump(envelope, stream, sort_keys=True)
                    stream.flush(); os.fsync(stream.fileno())
                os.fsync(fd)
                names.append(name)
        deadline = time.monotonic() + 15
        acknowledged = 0
        for name in names:
            envelope = read_pending(fd, name)
            if not isinstance(envelope, dict) or set(envelope) != {"association", "event"} or envelope.get("association") != association:
                raise AdapterError("pending_association_mismatch")
            saved = envelope["event"]
            if (not isinstance(saved, dict)
                    or set(saved) != {"project", "session_id", "adapter_id", "event_id", "observed_tool", "exit_code"}
                    or saved["project"] != config["project"]
                    or saved["session_id"] != config["workflow_session_id"]
                    or saved["adapter_id"] != digest("codex-session\0" + config["local_session_id"])
                    or not isinstance(saved["event_id"], str)
                    or not re.fullmatch(r"[0-9a-f]{64}", saved["event_id"])
                    or name != digest(association + saved["event_id"]) + ".json"
                    or not isinstance(saved["observed_tool"], str)
                    or not re.fullmatch(r"[A-Za-z0-9_.:-]{1,64}", saved["observed_tool"])
                    or saved["exit_code"] is not None):
                raise AdapterError("invalid_pending_observation")
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise AdapterError("pending_delivery_deadline")
            sender(config, envelope["event"], min(5, remaining))
            os.unlink(name, dir_fd=fd)
            os.fsync(fd)
            acknowledged += 1
        return {"status": "recorded", "acknowledged": acknowledged}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", required=True, type=Path)
    parser.add_argument("--flush", action="store_true", help="Retry pending observations only; never business commands")
    args = parser.parse_args()
    try:
        config = load_config(args.config)
        event = None
        if not args.flush:
            raw = sys.stdin.buffer.read(MAX_BYTES + 1)
            if len(raw) > MAX_BYTES:
                raise AdapterError("input_too_large")
            event = observation(config, json.loads(raw))
            if event is None:
                print(json.dumps({"status": "ignored"})); return 0
        print(json.dumps(deliver(config, event)))
        return 0
    except AdapterError as error:
        print(json.dumps({"status": "incomplete", "reason": str(error)}), file=sys.stderr)
    except (OSError, ValueError, urllib.error.URLError):
        # Transport errors can include URLs/headers. Keep diagnostics fixed and
        # leave any saved event pending for the exact same-identity retry.
        print(json.dumps({"status": "incomplete", "reason": "recording_unconfirmed_check_pending_state"}), file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
