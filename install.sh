#!/bin/sh
# Install rustudio for the current user on Linux: builds the release binary,
# then adds it to your PATH, your app launcher and your icon theme.
#
#   ./install.sh               build and install into ~/.local
#   ./install.sh --no-build    install the binary already in target/release
#   ./install.sh --prefix DIR  install somewhere else (default: ~/.local)
#   ./install.sh --uninstall   remove everything this script installed
#
# Your places, settings, sign-in and the downloaded Studio content
# (~/.local/share/rustudio, ~/.config/rustudio) are never touched.
set -eu

prefix="$HOME/.local"
build=1
uninstall=0

while [ $# -gt 0 ]; do
    case "$1" in
        --prefix) [ $# -ge 2 ] || { echo "--prefix needs a folder" >&2; exit 1; }
                  prefix="$2"; shift ;;
        --prefix=*) prefix="${1#--prefix=}" ;;
        --no-build) build=0 ;;
        --uninstall) uninstall=1 ;;
        -h|--help) sed -n '2,11p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown option: $1 (see --help)" >&2; exit 1 ;;
    esac
    shift
done

repo=$(cd "$(dirname "$0")" && pwd)
bindir="$prefix/bin"
libdir="$prefix/lib/rustudio"
appdir="$prefix/share/applications"
icondir="$prefix/share/icons/hicolor"
sizes="16 24 32 48 64 128 256 512"

refresh() {
    # Let the desktop notice the launcher and icons; all of these are optional.
    command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q "$appdir" 2>/dev/null || true
    command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t "$icondir" 2>/dev/null || true
    for k in kbuildsycoca6 kbuildsycoca5; do
        command -v "$k" >/dev/null 2>&1 && { "$k" >/dev/null 2>&1 || true; break; }
    done
    return 0
}

if [ "$uninstall" = 1 ]; then
    rm -f "$bindir/rustudio" "$appdir/rustudio.desktop"
    rm -rf "$libdir"
    for s in $sizes; do rm -f "$icondir/${s}x${s}/apps/rustudio.png"; done
    rm -f "$icondir/scalable/apps/rustudio.svg"
    refresh
    echo "rustudio uninstalled from $prefix"
    exit 0
fi

if [ "$build" = 1 ]; then
    cargo=$(command -v cargo || true)
    [ -n "$cargo" ] || [ ! -x "$HOME/.cargo/bin/cargo" ] || cargo="$HOME/.cargo/bin/cargo"
    if [ -z "$cargo" ]; then
        echo "cargo not found: install Rust 1.85+ (https://rustup.rs) and try again" >&2
        exit 1
    fi
    (cd "$repo" && "$cargo" build --release)
fi

bin="$repo/target/release/rustudio"
if [ ! -x "$bin" ]; then
    echo "no build found at $bin (run without --no-build)" >&2
    exit 1
fi

# The binary and the API dump it reads at runtime live together, so the
# install keeps working after this folder is moved or deleted.
mkdir -p "$bindir" "$libdir" "$appdir"
install -m 755 "$bin" "$libdir/rustudio"
install -m 644 "$repo/api-dump.json" "$libdir/api-dump.json"

cat > "$bindir/rustudio" <<EOF
#!/bin/sh
# Installed by rustudio's install.sh; ./install.sh --uninstall removes it.
: "\${RUSTUDIO_API_DUMP:=$libdir/api-dump.json}"
export RUSTUDIO_API_DUMP
exec "$libdir/rustudio" "\$@"
EOF
chmod 755 "$bindir/rustudio"

for s in $sizes; do
    install -D -m 644 "$repo/assets/icons/rustudio-$s.png" "$icondir/${s}x${s}/apps/rustudio.png"
done
install -D -m 644 "$repo/assets/icons/rustudio.svg" "$icondir/scalable/apps/rustudio.svg"

# Point the launcher at the full path, so it works even when $bindir
# isn't on the desktop session's PATH.
sed "s|^Exec=rustudio|Exec=\"$bindir/rustudio\"|" "$repo/assets/linux/rustudio.desktop" > "$appdir/rustudio.desktop"
chmod 644 "$appdir/rustudio.desktop"

refresh

echo "rustudio installed to $prefix"
case ":$PATH:" in
    *":$bindir:"*) echo "run it from your app launcher, or type: rustudio" ;;
    *) echo "run it from your app launcher, or add $bindir to your PATH to type: rustudio" ;;
esac
