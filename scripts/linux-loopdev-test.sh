#!/usr/bin/env bash
# Runs the ignored LinuxBlockDevice integration test against real loop
# devices (512-byte and 4096-byte logical sectors). Needs sudo for losetup;
# the test itself runs as the current user after the device is made readable.
set -euo pipefail
cd "$(dirname "$0")/.."

work=$(mktemp -d)
devs=()
cleanup() {
  for d in "${devs[@]}"; do sudo losetup -d "$d" || true; done
  rm -rf "$work"
}
trap cleanup EXIT

cargo test -p sf-core --test linux_loopdev --no-run

for sector in 512 4096; do
  img="$work/disk-$sector.img"
  head -c $((16 * 1024 * 1024)) /dev/urandom > "$img"
  dev=$(sudo losetup --find --show --sector-size "$sector" --direct-io=on "$img")
  devs+=("$dev")
  sudo chmod a+r "$dev"
  echo "== $dev ($sector-byte sectors)"
  SF_LOOPDEV="$dev" SF_LOOPDEV_BACKING="$img" SF_LOOPDEV_SECTOR="$sector" \
    cargo test -p sf-core --test linux_loopdev -- --ignored --nocapture
done
echo "loop device tests passed"
