#!/usr/bin/env bash
# Install (or update) Multi Cursor into ~/Applications (no sudo).
# By default copies an existing release build; pass --build to build first.
#
# Create and update paths both run the same Spotlight / Launch Services cleanup.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

DEST_DIR="${HOME}/Applications"
APP_NAME="Multi Cursor.app"
SRC="${ROOT}/src-tauri/target/release/bundle/macos/${APP_NAME}"
DEST="${DEST_DIR}/${APP_NAME}"
TARGET_DIR="${ROOT}/src-tauri/target"
BUNDLE_ID="local.multi-cursor.launcher"
LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
DO_BUILD=0

for arg in "$@"; do
  case "${arg}" in
    --build|-b) DO_BUILD=1 ;;
    -h|--help)
      echo "Usage: $(basename "$0") [--build]"
      echo "  --build  Run \`npm run tauri build\` before installing"
      echo "Default: install from an existing release bundle (no rebuild)."
      exit 0
      ;;
    *)
      echo "error: unknown argument: ${arg}" >&2
      echo "Usage: $(basename "$0") [--build]" >&2
      exit 1
      ;;
  esac
done

app_is_running() {
  if ps -axc -o comm= | grep -qx "multi-cursor"; then
    return 0
  fi
  if [[ -d "${DEST}" ]]; then
    local dest_resolved
    dest_resolved="$(cd "${DEST}" && pwd -P)"
    if ps -ax -o args= | grep -F "${dest_resolved}/Contents/MacOS/" | grep -vq grep; then
      return 0
    fi
  fi
  return 1
}

exclude_target_from_spotlight() {
  mkdir -p "${TARGET_DIR}"
  touch "${TARGET_DIR}/.metadata_never_index"
}

unregister_path() {
  local path="$1"
  [[ -n "${path}" ]] || return 0
  if [[ -x "${LSREGISTER}" ]]; then
    "${LSREGISTER}" -u "${path}" >/dev/null 2>&1 || true
  fi
}

# Unregister every Multi Cursor.app except the one in ~/Applications (create + update).
hide_undesired_apps() {
  local path dest_resolved
  dest_resolved="$(cd "${DEST_DIR}" && pwd -P)/${APP_NAME}"

  exclude_target_from_spotlight

  # 1) Explicit build artifact
  unregister_path "${SRC}"

  # 2) Anything Spotlight still knows with our bundle id
  if command -v mdfind >/dev/null 2>&1; then
    while IFS= read -r path; do
      [[ -z "${path}" ]] && continue
      if [[ "${path}" == "${DEST}" || "${path}" == "${dest_resolved}" ]]; then
        continue
      fi
      echo "  unregistering ${path}"
      unregister_path "${path}"
    done < <(mdfind "kMDItemCFBundleIdentifier == '${BUNDLE_ID}'" 2>/dev/null || true)
  fi

  # 3) Any .app / .dmg left under the Cargo target tree
  if [[ -d "${TARGET_DIR}" ]]; then
    while IFS= read -r path; do
      [[ -z "${path}" ]] && continue
      echo "  removing build artifact ${path}"
      unregister_path "${path}"
      rm -rf "${path}"
    done < <(find "${TARGET_DIR}" -name "${APP_NAME}" -type d 2>/dev/null || true)

    find "${TARGET_DIR}/release/bundle" -name '*.dmg' -type f -print -delete 2>/dev/null || true
  fi

  # 4) Register the installed copy and import into Spotlight
  if [[ -x "${LSREGISTER}" && -d "${DEST}" ]]; then
    "${LSREGISTER}" -f "${DEST}" >/dev/null 2>&1 || true
  fi
  if [[ -d "${DEST}" ]]; then
    mdimport "${DEST}" >/dev/null 2>&1 || true
  fi
}

if app_is_running; then
  echo "error: Multi Cursor is running." >&2
  echo "Quit it completely, then run install-app again." >&2
  exit 1
fi

exclude_target_from_spotlight

if [[ "${DO_BUILD}" -eq 1 ]]; then
  echo "Building release app…"
  npm run tauri build
  exclude_target_from_spotlight
fi

if [[ ! -d "${SRC}" ]]; then
  echo "error: built app not found at ${SRC}" >&2
  echo "Build once with: npm run tauri build" >&2
  echo "Or build and install in one step: npm run install-app -- --build" >&2
  exit 1
fi

if app_is_running; then
  echo "error: Multi Cursor was started during the build." >&2
  echo "Quit it completely, then run install-app again." >&2
  exit 1
fi

mkdir -p "${DEST_DIR}"
if [[ -d "${DEST}" ]]; then
  echo "Updating existing install at ${DEST}…"
  rm -rf "${DEST}"
else
  echo "Installing to ${DEST}…"
fi
cp -R "${SRC}" "${DEST}"

xattr -dr com.apple.quarantine "${DEST}" 2>/dev/null || true

echo "Cleaning Spotlight / Launch Services duplicates…"
hide_undesired_apps

echo "Installed to ${DEST}"
echo "Open with: open \"${DEST}\""
echo "Note: the build .app under src-tauri/target was removed after install so Spotlight only sees ~/Applications. Run tauri build again if you need a fresh bundle."
