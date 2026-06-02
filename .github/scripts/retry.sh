#!/usr/bin/env bash
set -euo pipefail

max_attempts=3

for attempt in $(seq 1 "$max_attempts"); do
  if "$@"; then
    exit 0
  else
    status=$?
  fi

  if [ "$attempt" -eq "$max_attempts" ]; then
    exit "$status"
  fi

  # GitHub-hosted runners can see transient registry/CDN failures, so retry only
  # after the command has failed instead of hiding the first failure's output.
  sleep $((attempt * 10))
done
