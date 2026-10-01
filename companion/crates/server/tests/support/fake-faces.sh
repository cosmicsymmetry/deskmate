#!/bin/sh
# A stand-in for `bun run companion/faces/src/main.ts`, so the Rust suite exercises the
# real subprocess seam -- spawn, stdin, stdout, exit codes, timeout -- without needing
# Bun or the network. It speaks the same two verbs. `render` is steered by words in
# the settings it is handed, which is how a test asks for a particular failure.
here=$(dirname "$0")
case "$1" in
describe)
	cat <<'JSON'
[
  {"kind": "weather", "label": "Weather", "fields": [
    {"type": "text", "key": "location", "label": "Location", "placeholder": "Dubai"},
    {"type": "enum", "key": "units", "label": "Units", "default": "metric", "options": [
      {"value": "metric", "label": "Metric"}, {"value": "imperial", "label": "Imperial"}]}]},
  {"kind": "rss", "label": "RSS feed", "fields": [
    {"type": "url", "key": "url", "label": "Feed URL", "placeholder": "https://example.com/feed.xml"},
    {"type": "text", "key": "title", "label": "Title", "placeholder": "News"}]},
  {"kind": "token", "label": "Token price", "fields": [
    {"type": "text", "key": "coin_id", "label": "Coin ID", "placeholder": "solana"},
    {"type": "text", "key": "currency", "label": "Currency", "placeholder": "usd", "default": "usd"}]},
  {"kind": "headlines", "label": "Headlines", "tap": "Tap the panel for the next stories.", "fields": [
    {"type": "enum", "key": "list", "label": "Stories", "default": "top", "options": [
      {"value": "top", "label": "Front page"}, {"value": "new", "label": "Newest"}]}]}
]
JSON
	;;
render)
	request=$(cat)
	if [ -n "${DESKMATE_FAKE_FACES_REQUEST_DIR:-}" ]; then
		printf '%s\n' "$request" >> "$DESKMATE_FAKE_FACES_REQUEST_DIR/requests.jsonl"
		case "$request" in
		*'"event":'*)
			if [ "${DESKMATE_FAKE_FACES_BLOCK_TAPS:-}" = "1" ]; then
				: > "$DESKMATE_FAKE_FACES_REQUEST_DIR/render-blocked"
				while [ ! -e "$DESKMATE_FAKE_FACES_REQUEST_DIR/release" ]; do
					sleep 0.01
				done
			fi
			;;
		esac
	fi
	if [ "${DESKMATE_FAKE_FACES_FAIL_SCHEDULED:-}" = "1" ]; then
		case "$request" in
		*'"event":'*) ;;
		*) echo "scheduled render failed for the routing test" >&2; exit 1 ;;
		esac
	fi
	case "$request" in
	*refuse-as-configuration*) echo "the coin was not found; check the coin ID" >&2; exit 2 ;;
	*refuse-as-transient*) echo "api.example returned HTTP 503" >&2; exit 1 ;;
	*answer-with-an-envelope*) printf '{"png":"%s","state":{"page":3}}' "$(base64 < "$here/fake-face.png" | tr -d '\n')" ;;
	*answer-with-a-null-state*) printf '{"png":"%s","state":null}' "$(base64 < "$here/fake-face.png" | tr -d '\n')" ;;
	*echo-the-request*) echo "$request" >&2; exit 1 ;;
	*answer-with-bad-base64*) printf '{"png":"not base64 at all!!","state":null}' ;;
	*answer-with-garbage*) echo "this is not a PNG" ;;
	*answer-with-wrong-size*) cat "$here/fake-face-wrong-size.png" ;;
	*answer-with-a-flood*) head -c 3000000 /dev/zero ;;
	*never-answer*) exec sleep 600 ;;
	*echo-the-environment*) env >&2; exit 1 ;;
	*echo-the-config-dir*) printf 'DESKMATE_CONFIG_DIR=%s\n' "${DESKMATE_CONFIG_DIR:-<unset>}" >&2; exit 1 ;;
	*echo-the-secrets-file*)
		# Stands in for a plugin reading its own credential via `readPluginSecrets`
		# (`companion/faces/src/plugins/secrets.ts`): read
		# `$DESKMATE_CONFIG_DIR/plugin-secrets.json` and report its raw bytes. Used
		# to prove two accounts' renders never see each other's file, not just that
		# some directory reached the child.
		if [ -n "${DESKMATE_CONFIG_DIR:-}" ] && [ -f "$DESKMATE_CONFIG_DIR/plugin-secrets.json" ]; then
			printf 'SECRETS=%s\n' "$(cat "$DESKMATE_CONFIG_DIR/plugin-secrets.json")" >&2
		else
			printf 'SECRETS=<none>\n' >&2
		fi
		exit 1
		;;
	*alternate-frame*) cat "$here/fake-face-alt.png" ;;
	*'"view":"page-1"'*) cat "$here/fake-face-alt.png" ;;
	*'"event":'*) printf '{"png":"%s","state":{"page":3}}' "$(base64 < "$here/fake-face.png" | tr -d '\n')" ;;
	*) cat "$here/fake-face.png" ;;
	esac
	;;
views)
	request=$(cat)
	if [ -n "${DESKMATE_FAKE_FACES_REQUEST_DIR:-}" ]; then
		printf '%s\n' "$request" >> "$DESKMATE_FAKE_FACES_REQUEST_DIR/plans.jsonl"
	fi
	case "$request" in
	# Only the tappable face offers a second view, matching `describe` above.
	*'"kind":"headlines"'*) printf '{"views":["","page-1"]}' ;;
	*) printf '{"views":[""]}' ;;
	esac
	;;
tap)
	request=$(cat)
	if [ -n "${DESKMATE_FAKE_FACES_REQUEST_DIR:-}" ]; then
		printf '%s\n' "$request" >> "$DESKMATE_FAKE_FACES_REQUEST_DIR/plans.jsonl"
	fi
	# The blocked-render fixture exists to exercise the render path, so a tap
	# there must fall back to one rather than being answered from a staged view.
	if [ "${DESKMATE_FAKE_FACES_BLOCK_TAPS:-}" = "1" ]; then
		echo "this fixture sends taps down the render path" >&2
		exit 1
	fi
	case "$request" in
	*refuse-the-tap*) echo "the tap could not be resolved" >&2; exit 1 ;;
	*'"kind":"headlines"'*) printf '{"view":"page-1","state":{"page":1}}' ;;
	*) printf '{"view":""}' ;;
	esac
	;;
*)
	echo "usage: fake-faces.sh describe | render | views | tap" >&2
	exit 64
	;;
esac
