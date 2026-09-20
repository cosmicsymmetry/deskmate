#!/usr/bin/env bash
#
# Deploy the Deskmate server and its browser companion to docker-vm.
#
# This exists because the runbook's prose version was correct and still slow: the
# container it launched kept CARGO_HOME and RUSTUP_HOME inside the image, so
# `--rm` threw away the crate registry after every run. A no-op deploy re-downloaded
# 229 crates and re-synced the toolchain to compile nothing -- 96 seconds of work to
# produce a byte-identical binary. With both homes mounted from the VM it is 0.7s.
#
# The other half is `rsync --checksum`. `git archive` stamps every file with the
# commit time, so a fresh export after any commit gives every file a new mtime and
# cargo rebuilds all seven workspace crates whether or not their contents moved.
# Checksum mode skips files whose content matches, and a skipped file keeps its old
# mtime -- so only the crates actually edited recompile.
#
# Usage:
#   deploy.sh              binary + UI + faces
#   deploy.sh --ui-only    just the browser companion (no Rust build, no restart)
#   deploy.sh --faces-only just the server-rendered faces (no Rust build, no restart)
#   deploy.sh --dry-run    export and sync, build, but do not install or restart
set -euo pipefail

VM=rodion@100.93.166.123
# Relative to the remote home on purpose. `~/deskmate-build` would expand on THIS
# machine at assignment time and send the Mac's path to the VM; both ssh and rsync
# resolve a relative path against the remote home, which is what was meant.
REMOTE=deskmate-build
IMAGE=rust:1.98-bookworm
WEB_DIR=/var/lib/private/deskmate/web
# The faces package (companion/faces): TypeScript the server runs as a subprocess.
# Like the web dist it is read at use, so shipping a face is an rsync. The unit sees
# this directory as /var/lib/deskmate/faces -- DynamicUser maps the private path.
FACES_DIR=/var/lib/private/deskmate/faces
FACES_DIR_IN_UNIT=/var/lib/deskmate/faces
BUN=/usr/local/bin/bun

ui_only=false
faces_only=false
dry_run=false
for arg in "$@"; do
	case "$arg" in
	--ui-only) ui_only=true ;;
	--faces-only) faces_only=true ;;
	--dry-run) dry_run=true ;;
	*)
		echo "unknown option: $arg" >&2
		exit 2
		;;
	esac
done

repo_root=$(git rev-parse --show-toplevel)
cd "$repo_root"

if [ -n "$(git status --porcelain)" ]; then
	# The export comes from HEAD, so a dirty tree would deploy code that is not
	# the code you are looking at. Refuse rather than silently ship the commit.
	echo "refusing to deploy: the working tree is dirty, and the export is from HEAD" >&2
	git status --short >&2
	exit 1
fi

say() { printf '\n\033[1m%s\033[0m\n' "$*"; }

# --- the faces --------------------------------------------------------------
ship_faces() {
	say "shipping the faces"
	# The gates run here, on the machine that has the sources: a face that does not
	# type-check or whose golden moved must not reach the panel.
	(cd companion/faces && bun install --frozen-lockfile >/dev/null 2>&1 && bun run check >/dev/null 2>&1 && bun test >/dev/null 2>&1) ||
		{
			echo "refusing to ship: companion/faces does not pass its own gates" >&2
			exit 1
		}
	# shellcheck disable=SC2029  # $BUN is meant to expand here.
	ssh "$VM" "test -x $BUN" || {
		cat >&2 <<-MISSING
			refusing to ship: $BUN is not on the VM. Once, on the VM:
			  curl -fsSL https://bun.sh/install | bash && sudo install -m 0755 ~/.bun/bin/bun $BUN
		MISSING
		exit 1
	}
	# node_modules is NOT synced: @resvg/resvg-js ships one native binary per
	# platform, and the Mac's is darwin-arm64. It is installed on the VM instead, and
	# `--exclude` keeps rsync's --delete from taking it between deploys.
	rsync -a --delete --exclude node_modules/ --exclude out/ \
		companion/faces/ "$VM:/tmp/deskmate-faces/"
	# shellcheck disable=SC2029  # $BUN is meant to expand here.
	ssh "$VM" "set -e
		cd /tmp/deskmate-faces
		$BUN install --frozen-lockfile --production >/dev/null
		# The suite again, HERE: the goldens are byte-exact and this is the platform
		# that draws for the panel, with its own native resvg build. Then the cold
		# start the server performs. A package that fails either must not replace
		# one that works.
		$BUN test >/dev/null 2>&1 || { echo 'the faces suite fails on the VM' >&2; exit 1; }
		$BUN run src/main.ts describe >/dev/null"

	if [ "$dry_run" = false ]; then
		# shellcheck disable=SC2029  # the variables are meant to expand here.
		ssh "$VM" "set -e
			sudo -n rsync -a --delete --chown=deskmate-server:deskmate-server /tmp/deskmate-faces/ $FACES_DIR/
			if ! sudo -n grep -q '^DESKMATE_FACES_DIR=' /etc/deskmate/server.env; then
				echo 'DESKMATE_FACES_DIR=$FACES_DIR_IN_UNIT' | sudo -n tee -a /etc/deskmate/server.env >/dev/null
				echo '  added DESKMATE_FACES_DIR to /etc/deskmate/server.env -- it takes effect at the next restart'
			fi"
		say "faces shipped (run per refresh, catalog re-read every minute -- no restart needed)"
	fi
}

if [ "$ui_only" = false ]; then
	ship_faces
fi
if [ "$faces_only" = true ]; then
	exit 0
fi

# --- the browser companion -------------------------------------------------
say "building the companion"
(cd companion/apps/deskmate && bun run build >/dev/null)
rsync -a --delete companion/apps/deskmate/dist/ "$VM:/tmp/deskmate-web/"

if [ "$dry_run" = false ]; then
	# shellcheck disable=SC2029  # $WEB_DIR is meant to expand here.
	ssh "$VM" "sudo -n rsync -a --delete --chown=deskmate-server:deskmate-server /tmp/deskmate-web/ $WEB_DIR/"
	say "companion shipped (served per request -- no restart needed)"
fi

if [ "$ui_only" = true ]; then
	exit 0
fi

# --- the binary ------------------------------------------------------------
say "exporting HEAD"
export_dir=$(mktemp -d)
trap 'rm -rf "$export_dir"' EXIT
# `firmware` too: the server compiles the panel's own LVGL for card previews.
git archive HEAD companion firmware | tar -x -C "$export_dir"

say "syncing sources"
# --checksum: see the header. --exclude target/: it is the 1.6 GB build cache and
# --delete would take it, turning every deploy back into a cold build.
rsync -a --checksum --delete --exclude 'target/' \
	"$export_dir/companion/" "$VM:$REMOTE/companion/"
rsync -a --checksum --delete --exclude 'managed_components/' \
	"$export_dir/firmware/" "$VM:$REMOTE/firmware/"
# managed_components/ is gitignored, so it is not in the export at all. It only
# changes when firmware/dependencies.lock does, and rsync makes that cheap to
# assert every time rather than remember.
rsync -a --checksum --delete \
	--exclude 'tests/' --exclude 'demos/' --exclude 'docs/' \
	--exclude 'scripts/' --exclude 'examples/' \
	firmware/managed_components/lvgl__lvgl/ \
	"$VM:$REMOTE/firmware/managed_components/lvgl__lvgl/"
rsync -a --checksum --delete \
	firmware/managed_components/espressif__cbor/ \
	"$VM:$REMOTE/firmware/managed_components/espressif__cbor/"

say "building in $IMAGE"
# shellcheck disable=SC2029  # $REMOTE and $IMAGE are meant to expand here.
ssh "$VM" "cd $REMOTE && sudo -n docker run --rm \
	-v \"\$PWD\":/work \
	-v \"\$PWD/.cargo\":/cargo \
	-v \"\$PWD/.rustup\":/rustup \
	-e CARGO_HOME=/cargo -e RUSTUP_HOME=/rustup \
	-w /work/companion $IMAGE \
	cargo build --release --locked -p server"

if [ "$dry_run" = true ]; then
	say "dry run: built, not installed"
	exit 0
fi

say "installing"
# shellcheck disable=SC2029  # $REMOTE is meant to expand here.
# The bin target is `server`; the installed file is renamed on the way in. The
# backup carries a UTC timestamp because more than one deploy a day is normal and
# `cp -a` overwrites silently -- a date-only name once destroyed the same day's
# only rollback target while reporting success.
ssh "$VM" "set -e
	if sudo -n cmp -s $REMOTE/companion/target/release/server /usr/local/bin/deskmate-server; then
		echo '  binary unchanged -- not restarting'
		exit 0
	fi
	sudo -n cp -a /usr/local/bin/deskmate-server \
		\"/usr/local/bin/deskmate-server.bak-\$(date -u +%Y%m%dT%H%M%SZ)\"
	sudo -n install -m 0755 $REMOTE/companion/target/release/server /usr/local/bin/deskmate-server
	sudo -n systemctl restart deskmate-server
	sleep 2
	sudo -n systemctl is-active deskmate-server"

say "checking"
curl -sf -o /dev/null -w '  GET /            %{http_code}\n' https://deskmate.rodi.one/
curl -s -o /dev/null -w '  GET /v1/app/...  %{http_code} (401 expected: no session)\n' \
	https://deskmate.rodi.one/v1/app/devices
