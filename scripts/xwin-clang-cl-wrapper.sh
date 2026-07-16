#!/usr/bin/env bash
set -euo pipefail

for arg in "$@"; do
  if [[ "$arg" == *"src/generate_version.c" ]]; then
    output=""
    for candidate in "$@"; do
      case "$candidate" in
        /OUT:*) output=${candidate#/OUT:} ;;
      esac
    done
    if [[ -z "$output" ]]; then
      echo "dokan generator wrapper could not locate /OUT" >&2
      exit 1
    fi
    mkdir -p "$(dirname "$output")"
    printf '%s\n' '#!/usr/bin/env bash' \
      'cat > version.rs <<'"'"'EOF'"'"'' \
      'pub const DOKAN_VERSION: u32 = 206;' \
      'pub const DOKAN_MINIMUM_COMPATIBLE_VERSION: u32 = 200;' \
      'pub const DOKAN_DRIVER_NAME: &str = "dokan2.sys";' \
      'pub const DOKAN_NP_NAME: &str = "Dokan2";' \
      'pub const DOKAN_MAJOR_API_VERSION: &str = "2";' \
      'EOF' \
      'printf 206 > version.txt' > "$output"
    chmod +x "$output"
    exit 0
  fi
done

exec "${TRAIL_REAL_CLANG_CL:-clang-cl}" "$@"
