# Source me: lets plain cargo (clippy, test --no-run) target Windows from WSL with zig,
# reusing the wrappers `cargo zigbuild --target x86_64-pc-windows-gnu` generates (run it
# once first). Local development aid only.
_zw=$(ls -d "$HOME"/.cache/cargo-zigbuild/*/wrappers/* 2>/dev/null | tail -1)
if [ -z "$_zw" ]; then
  echo "run 'cargo zigbuild --target x86_64-pc-windows-gnu' once first" >&2
else
  export PATH="$HOME/.local/bin:$_zw:$PATH"
  export CC_x86_64_pc_windows_gnu="$(ls "$_zw"/zigcc-x86_64-pc-windows-gnu-*.sh | head -1)"
  export AR_x86_64_pc_windows_gnu="$_zw/ar"
  export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="$CC_x86_64_pc_windows_gnu"
fi
unset _zw
