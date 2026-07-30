#!/bin/sh

status_json=$(cat)
rate_limits_json=$(printf '%s' "$status_json" | jq -c '{rate_limits: (.rate_limits // null)}' 2>/dev/null) || exit 0
usage_source_url=${AGENT_USAGE_SOURCE_URL:-http://127.0.0.1:9469}

if [ "$rate_limits_json" != '{"rate_limits":null}' ]; then
	printf '%s' "$rate_limits_json" |
		curl --fail --silent --show-error \
			--connect-timeout 0.2 \
			--max-time 0.5 \
			--header 'Content-Type: application/json' \
			--data-binary @- \
			"${usage_source_url}/v1/rate-limits/claude" \
			>/dev/null 2>&1 || true
fi

display=$(printf '%s' "$rate_limits_json" | jq -r '
  [
    if .rate_limits.five_hour.used_percentage != null
    then "5h: \(.rate_limits.five_hour.used_percentage | round)%"
    else empty end,
    if .rate_limits.seven_day.used_percentage != null
    then "7d: \(.rate_limits.seven_day.used_percentage | round)%"
    else empty end
  ] | join(" | ")
' 2>/dev/null)

if [ -n "$display" ]; then
	printf 'Claude %s\n' "$display"
fi
