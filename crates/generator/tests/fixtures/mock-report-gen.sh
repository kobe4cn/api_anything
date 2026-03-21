#!/bin/bash
# Mock CLI script that simulates a report generation tool.
# Used in E2E tests to avoid depending on a real external binary.
case "$1" in
    generate)
        shift
        # Extract --type value from args (next arg after --type flag)
        report_type=""
        while [[ $# -gt 0 ]]; do
            if [[ "$1" == "--type" ]]; then
                report_type="$2"
                shift 2
            else
                shift
            fi
        done
        echo "{\"report_id\": \"R-001\", \"status\": \"generated\", \"type\": \"$report_type\"}"
        ;;
    list)
        echo "[{\"id\": \"R-001\", \"date\": \"2024-01-01\"}, {\"id\": \"R-002\", \"date\": \"2024-01-02\"}]"
        ;;
    *)
        echo "Unknown subcommand: $1" >&2
        exit 1
        ;;
esac
