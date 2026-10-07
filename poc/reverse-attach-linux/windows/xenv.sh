# Source me: lets plain cargo (clippy, test --no-run) target Windows from WSL with zig,
# reusing the wrappers cargo-zigbuild generated. Local development aid only.
export PATH="$HOME/.local/bin:/home/nickhuang23/.cache/cargo-zigbuild/0.23.4/wrappers/2205:$PATH"
export CC_x86_64_pc_windows_gnu="/home/nickhuang23/.cache/cargo-zigbuild/0.23.4/wrappers/2205/zigcc-x86_64-pc-windows-gnu-d9a9.sh"
export AR_x86_64_pc_windows_gnu="/home/nickhuang23/.cache/cargo-zigbuild/0.23.4/wrappers/2205/ar"
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="$CC_x86_64_pc_windows_gnu"
