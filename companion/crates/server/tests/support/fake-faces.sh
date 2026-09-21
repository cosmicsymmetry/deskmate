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
  {"kind": "headlines", "label": "Headlines", "fields": [
    {"type": "enum", "key": "list", "label": "Stories", "default": "top", "options": [
      {"value": "top", "label": "Front page"}, {"value": "new", "label": "Newest"}]}]}
]
JSON
	;;
render)
	request=$(cat)
	case "$request" in
	*refuse-as-configuration*) echo "the coin was not found; check the coin ID" >&2; exit 2 ;;
	*refuse-as-transient*) echo "api.example returned HTTP 503" >&2; exit 1 ;;
	*answer-with-garbage*) echo "this is not a PNG" ;;
	*answer-with-wrong-size*) cat "$here/fake-face-wrong-size.png" ;;
	*answer-with-a-flood*) head -c 3000000 /dev/zero ;;
	*never-answer*) exec sleep 600 ;;
	*echo-the-environment*) env >&2; exit 1 ;;
	*alternate-frame*) cat "$here/fake-face-alt.png" ;;
	*) cat "$here/fake-face.png" ;;
	esac
	;;
*)
	echo "usage: fake-faces.sh describe | render" >&2
	exit 64
	;;
esac
