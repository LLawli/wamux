# Source this to build for aarch64-linux-android with the Android NDK (#277).
#
# One place for the NDK wiring, so the release and the CI job build the same
# way: cc-rs (SQLite, mimalloc) and cargo's linker both need the NDK's clang,
# and rustc cannot find it on its own. API 24 is the oldest Android the binary
# targets (bionic has everything the daemon calls from there on).
#
# The NDK is found through the variable the environment already sets: GitHub's
# ubuntu images export ANDROID_NDK_LATEST_HOME, Android Studio and most
# toolchains use ANDROID_NDK_HOME or ANDROID_NDK_ROOT.
#
# Exports: ANDROID_NDK_BIN (the NDK's llvm bin directory), CC_/AR_ for the
# target, the cargo linker, and ANDROID_LLVM_STRIP (the host `strip` does not
# understand aarch64 ELF).

ANDROID_API_LEVEL=24

find_android_ndk() {
  local candidate
  for candidate in "${ANDROID_NDK_HOME:-}" "${ANDROID_NDK_LATEST_HOME:-}" "${ANDROID_NDK_ROOT:-}"; do
    if [[ -n "$candidate" && -d "$candidate/toolchains/llvm/prebuilt" ]]; then
      echo "$candidate"
      return 0
    fi
  done
  echo "ERROR: no Android NDK found: set ANDROID_NDK_HOME (or ANDROID_NDK_LATEST_HOME, ANDROID_NDK_ROOT) to a directory with toolchains/llvm/prebuilt" >&2
  return 1
}

use_android_ndk() {
  local ndk bin
  ndk=$(find_android_ndk) || return 1
  bin=$(echo "$ndk"/toolchains/llvm/prebuilt/*/bin)
  if [[ ! -x "$bin/aarch64-linux-android${ANDROID_API_LEVEL}-clang" ]]; then
    echo "ERROR: $bin has no aarch64-linux-android${ANDROID_API_LEVEL}-clang: the NDK at $ndk is too old or damaged" >&2
    return 1
  fi
  export ANDROID_NDK_BIN="$bin"
  export CC_aarch64_linux_android="$bin/aarch64-linux-android${ANDROID_API_LEVEL}-clang"
  export AR_aarch64_linux_android="$bin/llvm-ar"
  export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$CC_aarch64_linux_android"
  export ANDROID_LLVM_STRIP="$bin/llvm-strip"
}
