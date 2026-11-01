#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
export PATH="${HOME}/.cargo/bin:${PATH}"
# Use system development metadata when installed. On this Ubuntu machine,
# the pure Rust bindings can link the existing runtime libraries directly.
native_dir="$PWD/.work/native"
mkdir -p "$native_dir/lib" "$native_dir/pkgconfig"
for spec in 'fontconfig:libfontconfig.so.1:fontconfig:2.15.0' 'dbus-1:libdbus-1.so.3:dbus-1:1.14.10'; do
  IFS=: read -r package soname linkname version <<< "$spec"
  if ! pkg-config --exists "$package"; then
    runtime_path="$(ldconfig -p | awk -v library="$soname" '$1==library && /x86-64/ {print $NF; exit}')"
    if [[ -z "$runtime_path" ]]; then
      echo "Missing $package runtime. Install its development package, then rebuild." >&2
      exit 1
    fi
    ln -sf "$runtime_path" "$native_dir/lib/lib$linkname.so"
    cat > "$native_dir/pkgconfig/$package.pc" <<META
prefix=$native_dir
libdir=\${prefix}/lib
Name: $package
Description: System runtime linkage for pre-generated Rust bindings
Version: $version
Libs: -L\${libdir} -l$linkname
META
  fi
done
export PKG_CONFIG_PATH="$native_dir/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
exec cargo "${@:-build}" --release --locked
