#!/usr/bin/env bash
# Stage the MPI launcher and non-system shared libraries beside release kernels.
set -euo pipefail

KERNELS="${1:-kernels}"
RUNTIME="$KERNELS/_runtime"
mkdir -p "$RUNTIME/bin" "$RUNTIME/lib" "$RUNTIME/share" "$RUNTIME/etc"

copy_file() {
  local src=$1 dst=$2
  test -f "$src" || return 0
  cp -L "$src" "$dst"
  chmod u+w "$dst"
}

refresh_manifests() {
  local py
  py=$(command -v python3 || command -v python || true)
  test -n "$py" || { echo "python is required to refresh kernel manifests" >&2; exit 2; }
  "$py" - "$KERNELS" <<'PY'
import hashlib, json, pathlib, sys
root = pathlib.Path(sys.argv[1])
for manifest in root.glob('*/manifest.json'):
    data = json.loads(manifest.read_text())
    suffix = '.exe' if str(data.get('platform', '')).startswith(('MINGW', 'MSYS')) else '.x'
    hashes = {}
    for prog in ('mksrfdata', 'mkinidata', 'colm'):
        path = manifest.parent / f'{prog}{suffix}'
        if path.exists():
            hashes[prog] = hashlib.sha256(path.read_bytes()).hexdigest()
    if hashes:
        data['sha256'] = hashes
        manifest.write_text(json.dumps(data, indent=2, ensure_ascii=False) + '\n')
PY
}

case "$(uname -s)" in
  Darwin)
    ompi=$(brew --prefix open-mpi)
    prrte=$(brew --prefix prrte)
    copy_file "$ompi/bin/mpiexec" "$RUNTIME/bin/mpiexec"
    for name in prte prted prterun; do
      path=$(command -v "$name" || true)
      test -n "$path" && copy_file "$path" "$RUNTIME/bin/$name"
    done
    for prefix in "$ompi" "$prrte"; do
      for lib in "$prefix"/lib/*.dylib; do copy_file "$lib" "$RUNTIME/lib/$(basename "$lib")"; done
    done
    if test -d "$ompi/lib/openmpi"; then
      mkdir -p "$RUNTIME/lib/openmpi"
      cp -RL "$ompi/lib/openmpi/." "$RUNTIME/lib/openmpi/"
    fi
    if test -d "$ompi/share/openmpi"; then
      mkdir -p "$RUNTIME/share/openmpi"
      cp -RL "$ompi/share/openmpi/." "$RUNTIME/share/openmpi/"
    fi
    if test -d "$prrte/share/prte"; then
      mkdir -p "$RUNTIME/share/prte"
      cp -RL "$prrte/share/prte/." "$RUNTIME/share/prte/"
    fi

    # Close and rewrite every Homebrew dependency edge. The system loader paths
    # stay untouched; all other dylibs live in one private rpath directory.
    while :; do
      added=0
      while IFS= read -r file; do
        file "$file" | grep -q 'Mach-O' || continue
        while IFS= read -r dep; do
          case "$dep" in
            /System/*|/usr/lib/*|@loader_path/*|@executable_path/*) continue ;;
            @rpath/*)
              base=${dep##*/}
              test -f "$RUNTIME/lib/$base" && continue
              src=$(find "$ompi" "$prrte" "$(brew --prefix)" "$(brew --prefix)/opt" /usr/local -name "$base" -type f -print -quit 2>/dev/null || true)
              ;;
            *) base=${dep##*/}; src=$dep ;;
          esac
          if test -n "${src:-}" && test -f "$src" && test ! -f "$RUNTIME/lib/$base"; then
            copy_file "$src" "$RUNTIME/lib/$base"
            added=1
          fi
        done < <(otool -L "$file" | tail -n +2 | awk '{print $1}')
      done < <(find "$KERNELS" -type f)
      test "$added" -eq 1 || break
    done

    while IFS= read -r file; do
      file "$file" | grep -q 'Mach-O' || continue
      chmod u+w "$file"
      codesign --remove-signature "$file" >/dev/null 2>&1 || true
      if test -n "$(otool -D "$file" | tail -n +2)"; then
        install_name_tool -id "@rpath/$(basename "$file")" "$file"
      fi
      while IFS= read -r dep; do
        case "$dep" in
          /System/*|/usr/lib/*|@loader_path/*|@executable_path/*) continue ;;
        esac
        base=${dep##*/}
        test -f "$RUNTIME/lib/$base" && install_name_tool -change "$dep" "@rpath/$base" "$file"
      done < <(otool -L "$file" | tail -n +2 | awk '{print $1}')
      case "$file" in
        "$RUNTIME/bin/"*) install_name_tool -add_rpath '@executable_path/../lib' "$file" 2>/dev/null || true ;;
        "$RUNTIME/lib/"*)
          install_name_tool -add_rpath '@loader_path' "$file" 2>/dev/null || true
          install_name_tool -add_rpath '@loader_path/..' "$file" 2>/dev/null || true
          ;;
        *) install_name_tool -add_rpath '@executable_path/../_runtime/lib' "$file" 2>/dev/null || true ;;
      esac
      codesign --force --sign - "$file" >/dev/null 2>&1 || true
    done < <(find "$KERNELS" -type f)
    ;;

  Linux)
    launcher=$(command -v mpiexec.openmpi || command -v mpiexec)
    copy_file "$launcher" "$RUNTIME/bin/mpiexec"
    daemon=$(command -v orted || true)
    test -z "$daemon" || copy_file "$daemon" "$RUNTIME/bin/orted"
    libdir=$(ompi_info --path libdir --parsable | cut -d: -f3-)
    pkglibdir=$(ompi_info --path pkglibdir --parsable | cut -d: -f3-)
    datadir=$(ompi_info --path datadir --parsable | cut -d: -f3-)
    for lib in "$libdir"/*.so*; do copy_file "$lib" "$RUNTIME/lib/$(basename "$lib")"; done
    if test -d "$pkglibdir"; then
      mkdir -p "$RUNTIME/lib/openmpi"
      cp -RL "$pkglibdir/." "$RUNTIME/lib/openmpi/"
    fi
    if test -d "$datadir/openmpi"; then
      mkdir -p "$RUNTIME/share/openmpi"
      cp -RL "$datadir/openmpi/." "$RUNTIME/share/openmpi/"
    fi
    if test -d /etc/openmpi; then
      mkdir -p "$RUNTIME/etc/openmpi"
      cp -RL /etc/openmpi/. "$RUNTIME/etc/openmpi/"
    fi

    while IFS= read -r file; do
      file "$file" | grep -qE 'ELF .* (executable|shared object)' || continue
      while IFS= read -r dep; do
        base=$(basename "$dep")
        case "$base" in
          ld-linux*|libc.so*|libm.so*|libdl.so*|libpthread.so*|librt.so*|libutil.so*|libresolv.so*) continue ;;
        esac
        test -f "$RUNTIME/lib/$base" || copy_file "$dep" "$RUNTIME/lib/$base"
      done < <(ldd "$file" 2>/dev/null | awk '/=> \// {print $3} /^\// {print $1}')
    done < <(find "$KERNELS" -type f)
    ;;

  MINGW*|MSYS*)
    launcher=$(command -v mpiexec.exe || command -v mpiexec || true)
    if test -z "$launcher"; then
      for candidate in "/c/Program Files/Microsoft MPI/Bin/mpiexec.exe" /mingw64/bin/mpiexec.exe; do
        if test -f "$candidate"; then launcher=$candidate; break; fi
      done
    fi
    test -n "$launcher" || { echo "Microsoft MPI launcher not found" >&2; exit 2; }
    copy_file "$launcher" "$RUNTIME/bin/mpiexec.exe"
    daemon=$(command -v smpd.exe || true)
    test -n "$daemon" || daemon="$(dirname "$launcher")/smpd.exe"
    test -z "$daemon" || copy_file "$daemon" "$RUNTIME/bin/smpd.exe"
    for dll in /mingw64/bin/msmpi.dll /c/Windows/System32/msmpi.dll; do
      test ! -f "$dll" || copy_file "$dll" "$RUNTIME/bin/msmpi.dll"
    done
    for file in "$RUNTIME/bin"/*.exe; do
      ldd "$file" | awk 'tolower($0) ~ /\/mingw64\/bin\/.*\.dll/ {print $3}' | while read -r dll; do
        copy_file "$dll" "$RUNTIME/bin/$(basename "$dll")"
      done
    done
    ;;

  *)
    echo "unsupported MPI runtime platform: $(uname -s)" >&2
    exit 2
    ;;
esac

test -f "$RUNTIME/bin/mpiexec" || test -f "$RUNTIME/bin/mpiexec.exe"
refresh_manifests
echo "staged MPI runtime -> $RUNTIME"
