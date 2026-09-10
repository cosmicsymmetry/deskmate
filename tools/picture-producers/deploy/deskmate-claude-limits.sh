#!/bin/sh
# Renders the Claude usage panel and pushes it to its Deskmate image source.
#
# Deliberately separate from TRMNL's `claude_usage_sync.sh` even though both read
# the same feed: that script belongs to another repository, and a picture card is
# a Deskmate concern. The two are coupled only by the feed's URL.
#
# On failure this exits 0. The panel keeps its last good picture, and staleness is
# inferred from the pushes that DO arrive -- so a run that cannot reach the feed
# should leave no trace rather than record a push that never happened.
set -u

FEED_URL="${DESKMATE_CLAUDE_FEED_URL:?DESKMATE_CLAUDE_FEED_URL is required}"
TOKEN_FILE="${DESKMATE_IMAGE_TOKEN_FILE:-/etc/deskmate/producers/claude-limits.token}"
PUSH_BASE="${DESKMATE_PUSH_BASE:-https://deskmate.rodi.one/v1/images}"
LOG="${DESKMATE_PRODUCER_LOG:-/var/log/deskmate-claude-limits.log}"

ts() { date -u +%Y-%m-%dT%H:%M:%S; }

if [ ! -r "$TOKEN_FILE" ]; then
  echo "$(ts) token file $TOKEN_FILE is unreadable" >> "$LOG"
  exit 0
fi
TOKEN=$(cat "$TOKEN_FILE")

if ! /opt/deskmate-producers/claude_limits_png.py \
      --feed "$FEED_URL" \
      --push "$PUSH_BASE/$TOKEN" >> "$LOG" 2>&1; then
  echo "$(ts) push failed; keeping the last good picture" >> "$LOG"
  exit 0
fi
echo "$(ts) pushed" >> "$LOG"
