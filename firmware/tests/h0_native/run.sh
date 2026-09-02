#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
build_dir=$(mktemp -d "${TMPDIR:-/tmp}/agent-bridge-h0-native.XXXXXX")
trap 'rm -rf "$build_dir"' EXIT HUP INT TERM

cc -std=c11 -Wall -Wextra -Werror \
  -I"$root/components/abp/include" \
  -I"$root/main" \
  "$root/components/abp/abp.c" \
  "$root/main/h0_session.c" \
  "$root/main/h0_visual.c" \
  "$root/tests/h0_native/test_h0.c" \
  -o "$build_dir/test_h0"
"$build_dir/test_h0"
