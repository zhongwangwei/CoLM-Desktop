#!/usr/bin/env bash
set -euo pipefail
cargo install tauri-cli --version '^2' --locked
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
cargo tauri build "${config[@]}"
