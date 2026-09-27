#!/usr/bin/env python3
"""Read an explicitly selected WebCodex Workflow Session for local Codex recovery."""

import argparse
import json
from pathlib import Path
import sys
import urllib.error
import urllib.request

from external_observation_hook import AdapterError, MAX_BYTES, NoRedirect, load_config, private_file


def read_handoff(config, timeout=6, current_dir=None):
    # A private operator configuration selects the exact Project and Session.
    # The current directory can only narrow that selection, never redirect it.
    if not (current_dir or Path.cwd()).resolve().is_relative_to(Path(config["project_root"])):
        raise AdapterError("outside_configured_project")
    authorization = private_file(config["authorization_file"]).decode().strip()
    if not authorization or "\n" in authorization or "\r" in authorization:
        raise AdapterError("invalid_authorization_file")
    request = urllib.request.Request(
        config["server_url"].rstrip("/") + "/api/tools/call",
        data=json.dumps({
            "tool": "session_handoff_state",
            "params": {
                "project": config["project"],
                "session_id": config["workflow_session_id"],
            },
        }).encode(),
        headers={"Content-Type": "application/json", "Authorization": authorization},
        method="POST",
    )
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
    with opener.open(request, timeout=timeout) as response:
        raw = response.read(MAX_BYTES + 1)
    if len(raw) > MAX_BYTES:
        raise AdapterError("oversized_handoff_response")
    result = json.loads(raw)
    if not isinstance(result, dict) or result.get("success") is not True:
        raise AdapterError("handoff_read_failed")
    output = result.get("output")
    if (not isinstance(output, dict)
            or output.get("project") != config["project"]
            or output.get("session_id") != config["workflow_session_id"]
            or not isinstance(output.get("handoff_brief"), dict)):
        raise AdapterError("handoff_identity_mismatch")
    brief = output["handoff_brief"]
    if (not isinstance(brief.get("session"), dict)
            or brief["session"].get("session_id") != config["workflow_session_id"]):
        raise AdapterError("handoff_identity_mismatch")
    if (not isinstance(brief.get("basis"), dict)
            or not isinstance(brief["basis"].get("complete"), bool)):
        raise AdapterError("handoff_basis_missing")
    external = brief.get("external_observations")
    coverage = external.get("coverage") if isinstance(external, dict) else None
    if (not isinstance(external, dict)
            or external.get("provenance") != "external_report"
            or not isinstance(coverage, dict)
            or coverage.get("complete") is not False):
        raise AdapterError("handoff_external_observations_missing")
    if brief.get("deterministic") is not True or brief.get("llm_summary") is not False:
        raise AdapterError("handoff_contract_invalid")
    return {
        "status": "read",
        "project": output["project"],
        "session_id": output["session_id"],
        "handoff_brief": brief,
        "recovery_note": "Historical evidence only. Check current project rules, files, Git state and unresolved work before acting; never replay an unknown operation.",
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", required=True, type=Path)
    args = parser.parse_args()
    try:
        config = load_config(args.config)
        print(json.dumps(read_handoff(config), ensure_ascii=False, sort_keys=True))
        return 0
    except AdapterError as error:
        print(json.dumps({"status": "unavailable", "reason": str(error)}), file=sys.stderr)
    except (OSError, ValueError, UnicodeError, urllib.error.URLError):
        print(json.dumps({"status": "unavailable", "reason": "handoff_read_unconfirmed"}), file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
