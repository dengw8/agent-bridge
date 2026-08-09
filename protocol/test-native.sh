#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
build_dir=$(mktemp -d "${TMPDIR:-/tmp}/agent-bridge-abp.XXXXXX")
trap 'rm -rf "$build_dir"' EXIT HUP INT TERM

cc -std=c11 -Wall -Wextra -Werror -pedantic \
  -DABP_REPO_ROOT="\"$repo_root\"" \
  -I"$repo_root/firmware/components/abp/include" \
  "$repo_root/firmware/components/abp/abp.c" \
  "$repo_root/firmware/tests/abp_native/test_abp.c" \
  -o "$build_dir/test-abp"

"$build_dir/test-abp"
