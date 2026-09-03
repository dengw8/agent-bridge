#!/bin/sh
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$here/../.." && pwd)
repo=$(CDPATH= cd -- "$root/../.." && pwd)
binary="${TMPDIR:-/tmp}/agent-passport-avatar-native"

cc -std=c11 -Wall -Wextra -Werror \
  -I"$root/main" \
  -I"$repo/firmware/components/abp/include" \
  "$repo/firmware/components/abp/abp.c" \
  "$root/main/avatar_ble_frag.c" \
  "$root/main/avatar_display_packet.c" \
  "$root/main/avatar_model.c" \
  "$root/main/avatar_session.c" \
  "$here/test_avatar.c" \
  -o "$binary"

"$binary"
node "$here/check_font_coverage.mjs"
