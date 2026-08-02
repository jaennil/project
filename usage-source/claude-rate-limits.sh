#!/bin/sh

status_json=$(cat)
usage_source_url=${AGENT_USAGE_SOURCE_URL:-http://127.0.0.1:9469}
credentials_file=${CLAUDE_CREDENTIALS_FILE:-"${HOME}/.claude/.credentials.json"}
cache_seconds=${CLAUDE_RATE_LIMIT_CACHE_SECONDS:-30}
max_cache_age=${CLAUDE_RATE_LIMIT_MAX_CACHE_AGE:-120}
cache_directory=${CLAUDE_RATE_LIMIT_CACHE_DIR:-${XDG_RUNTIME_DIR:-/tmp}}

case "$cache_seconds" in
	'' | *[!0-9]*) cache_seconds=30 ;;
esac
case "$max_cache_age" in
	'' | *[!0-9]*) max_cache_age=120 ;;
esac

if ! mkdir -p "$cache_directory" 2>/dev/null; then
	cache_directory=/tmp
fi
user_id=$(id -u)
cache_file="${cache_directory}/claude-rate-limits-${user_id}.json"
lock_file="${cache_file}.lock"

cache_is_fresh() {
	maximum_age=$1
	modified_at=$(stat -c %Y "$cache_file" 2>/dev/null) || return 1
	now=$(date +%s)
	age=$((now - modified_at))
	[ "$age" -ge 0 ] && [ "$age" -le "$maximum_age" ]
}

refresh_cache() {
	(
		flock -n 9 || exit 0
		cache_is_fresh "$cache_seconds" && exit 0
		access_token=$(jq -er '.claudeAiOauth.accessToken | select(type == "string" and length > 0)' "$credentials_file" 2>/dev/null) || exit 0
		usage_json=$(
			{
				printf 'header = "Authorization: Bearer %s"\n' "$access_token"
				printf 'header = "anthropic-beta: oauth-2025-04-20"\n'
			} |
				curl --fail --silent --show-error \
					--connect-timeout 3 \
					--max-time 10 \
					--config - \
					https://api.anthropic.com/api/oauth/usage \
					2>/dev/null
		) || exit 0
		rate_limits_json=$(printf '%s' "$usage_json" | jq -ce '
      def clean_window:
        if type == "object"
          and (.utilization | type) == "number"
          and (.resets_at | type) == "string"
        then {utilization, resets_at}
        else null
        end;
      {
        five_hour: (.five_hour | clean_window),
        seven_day: (.seven_day | clean_window)
      }
      | with_entries(select(.value != null))
      | if length == 0 then error("rate limits missing") else . end
    ' 2>/dev/null) || exit 0
		temporary_file=$(mktemp "${cache_file}.tmp.XXXXXX") || exit 0
		if ! printf '%s\n' "$rate_limits_json" >"$temporary_file"; then
			rm -f "$temporary_file"
			exit 0
		fi
		mv "$temporary_file" "$cache_file"
	) 9>"$lock_file"
}

refresh_cache

rate_limits_json=
if cache_is_fresh "$max_cache_age"; then
	rate_limits_json=$(cat "$cache_file" 2>/dev/null)
else
	now=$(date +%s)
	rate_limits_json=$(printf '%s' "$status_json" | jq -ce --argjson now "$now" '
    def fresh_window:
      if type == "object"
        and (.used_percentage | type) == "number"
        and (.resets_at | type) == "number"
        and .resets_at > $now
      then {used_percentage, resets_at}
      else null
      end;
    {
      rate_limits: ({
        five_hour: (.rate_limits.five_hour | fresh_window),
        seven_day: (.rate_limits.seven_day | fresh_window)
      } | with_entries(select(.value != null)))
    }
    | if (.rate_limits | length) == 0 then error("rate limits missing") else . end
  ' 2>/dev/null) || rate_limits_json=
fi

if [ -n "$rate_limits_json" ]; then
	printf '%s' "$rate_limits_json" |
		curl --noproxy '*' --fail --silent --show-error \
			--connect-timeout 0.2 \
			--max-time 1 \
			--header 'Content-Type: application/json' \
			--data-binary @- \
			"${usage_source_url}/v1/rate-limits/claude" \
			>/dev/null 2>&1 || true
fi

display=$(printf '%s' "$rate_limits_json" | jq -r '
  [
    if .five_hour.utilization != null
    then "5h: \(.five_hour.utilization | round)%"
    elif .rate_limits.five_hour.used_percentage != null
    then "5h: \(.rate_limits.five_hour.used_percentage | round)%"
    else empty end,
    if .seven_day.utilization != null
    then "7d: \(.seven_day.utilization | round)%"
    elif .rate_limits.seven_day.used_percentage != null
    then "7d: \(.rate_limits.seven_day.used_percentage | round)%"
    else empty end
  ] | join(" | ")
' 2>/dev/null)

if [ -n "$display" ]; then
	printf 'Claude %s\n' "$display"
fi
