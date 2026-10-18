#!/usr/bin/env bash
set -euo pipefail
# rust-cache restores ~/.cargo/bin; only install when it did not.
command -v cargo-tauri >/dev/null 2>&1 || cargo install tauri-cli --version '^2' --locked
config=(--config tauri.bundle.conf.json)
if [[ "$RUNNER_OS" == Windows && "$RELEASE_PRERELEASE" == true ]]; then
  if [[ "$RELEASE_VERSION" =~ ^([0-9]+\.[0-9]+\.[0-9]+)-([0-9]+)$ ]]; then
    core=${BASH_REMATCH[1]}
    sequence=$((10#${BASH_REMATCH[2]}))
    (( sequence <= 9999 )) || {
      echo "::error::Numeric prerelease must not exceed 9999 for Windows installers"
      exit 1
    }
    wix_version="$core.$sequence"
  elif [[ "$RELEASE_VERSION" =~ ^([0-9]+\.[0-9]+\.[0-9]+)-(alpha|beta|rc)\.([0-9]+)$ ]]; then
    core=${BASH_REMATCH[1]}
    channel=${BASH_REMATCH[2]}
    sequence=$((10#${BASH_REMATCH[3]}))
    (( sequence <= 9999 )) || {
      echo "::error::Prerelease sequence must not exceed 9999 for Windows installers"
      exit 1
    }
    case "$channel" in
      alpha) offset=10000 ;;
      beta) offset=20000 ;;
      rc) offset=30000 ;;
    esac
    wix_version="$core.$((offset + sequence))"
  else
    echo "::error::Windows installers support numeric, alpha.N, beta.N, or rc.N prerelease versions"
    exit 1
  fi
  config+=(--config "{\"bundle\":{\"windows\":{\"wix\":{\"version\":\"$wix_version\"}}}}")
  echo "Packaging $RELEASE_VERSION with MSI ProductVersion $wix_version"
fi
if [[ "$RUNNER_OS" == Linux && "$RELEASE_VERSION" == *-* ]]; then
  # RPM versions may not contain "-" (newer tauri-cli refuses "0.3.0-beta.4"; beta.3 still
  # packaged). Build the deb and AppImage as usual, then the rpm as version 0.3.0 with release
  # 0.beta.4: a release starting with "0." sorts before the final 0.3.0-1, so the final release
  # still upgrades it ("0.3.0+beta.4" would sort after 0.3.0 and block that upgrade).
  core=${RELEASE_VERSION%%-*}
  pre=${RELEASE_VERSION#*-}
  rpm_release="0.${pre//-/.}"
  echo "Packaging $RELEASE_VERSION as rpm $core-$rpm_release"
  cargo tauri build "${config[@]}" --bundles deb,appimage
  cargo tauri build "${config[@]}" --bundles rpm \
    --config "{\"version\":\"$core\",\"bundle\":{\"linux\":{\"rpm\":{\"release\":\"$rpm_release\"}}}}"
else
  cargo tauri build "${config[@]}"
fi
