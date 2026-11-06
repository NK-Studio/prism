# shellcheck shell=bash
# Shared by package.sh and verify.sh: notarytool credentials from the environment. Source it after
# env.sh. Either set is enough; the API key wins when both are complete.
#
#   App Store Connect API key:
#     APPLE_API_KEY       the .p8 key, as a path to the file or as its contents (the CI secret)
#     APPLE_API_KEY_ID    key ID (the XXXX in AuthKey_XXXX.p8)
#     APPLE_API_ISSUER    issuer ID (App Store Connect > Users and Access > Integrations)
#   Apple ID:
#     APPLE_ID, APPLE_PASSWORD (app-specific password), APPLE_TEAM_ID
#
# Sets NOTARY_KIND to "api-key", "apple-id" or "" (incomplete). notary_auth fills NOTARY_AUTH with
# the matching notarytool arguments; a key given as contents is written to a private temp file that
# is removed on exit. stapler_validate retries `xcrun stapler validate`, whose CloudKit lookup drops
# the connection on CI runners now and then (error 68). Nothing here prints a secret's value.

NOTARY_KIND=""
if [ -n "${APPLE_API_KEY:-}" ] && [ -n "${APPLE_API_KEY_ID:-}" ] && [ -n "${APPLE_API_ISSUER:-}" ]; then
  NOTARY_KIND=api-key
elif [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_PASSWORD:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ]; then
  NOTARY_KIND=apple-id
fi

NOTARY_AUTH=()
notary_auth() {
  local key
  case "$NOTARY_KIND" in
    api-key)
      key="$APPLE_API_KEY"
      if [ ! -f "$key" ]; then
        key="$(umask 077 && mktemp "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/prism-notary.XXXXXX")"
        printf '%s\n' "$APPLE_API_KEY" >"$key"
        # shellcheck disable=SC2064  # expand $key now, the local is gone by exit
        trap "rm -f '$key'" EXIT
      fi
      NOTARY_AUTH=(--key "$key" --key-id "$APPLE_API_KEY_ID" --issuer "$APPLE_API_ISSUER")
      ;;
    apple-id)
      NOTARY_AUTH=(--apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID")
      ;;
  esac
}

# stapler_validate <app|dmg|pkg>: up to three attempts, 10 s apart; returns the last status.
stapler_validate() {
  local attempt
  for attempt in 1 2 3; do
    xcrun stapler validate "$1" && return 0
    [ "$attempt" = 3 ] && return 1
    echo "stapler validate failed (attempt $attempt of 3), retrying in 10 s" >&2
    sleep 10
  done
}
