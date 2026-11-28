# wamux on Android (Termux)

`wamux` runs on an Android phone inside [Termux](https://termux.dev), on an
aarch64 device with Android 7 (API 24) or newer. It is the same daemon as on a
server, built for Android's own C library (bionic): a static musl binary starts
there but cannot resolve names, because Android has no `/etc/resolv.conf`.

Run it **inside Termux**, as the Termux user. A shell started by `adb shell`
cannot: SELinux denies it binding a socket under `/data/local/tmp`.

## Install

From the `.deb`, which also installs the service definition:

```sh
curl -LO https://github.com/LLawli/wamux/releases/download/v<version>/wamux_<version>_aarch64.deb
apt install ./wamux_<version>_aarch64.deb
```

Or from the tarball, which holds the binary, the licenses and the same service
files under `contrib/termux/`:

```sh
curl -LO https://github.com/LLawli/wamux/releases/download/v<version>/wamux-<version>-aarch64-linux-android.tar.gz
tar -xzf wamux-<version>-aarch64-linux-android.tar.gz
cp wamux-<version>-aarch64-linux-android/wamux "$PREFIX/bin/"
```

Verify either with `gh attestation verify <file> -R LLawli/wamux`, as in
[Verifying a release](DEPLOYMENT.md#verifying-a-release).

## Defaults

The Termux build needs no `WAMUX_*` variable. Everything lives under Termux's
install prefix, `/data/data/com.termux/files/usr` (`$PREFIX`), which is the same
in every Termux:

| Setting | Default |
| --- | --- |
| `socket_path` | `$PREFIX/var/run/wamux.sock` |
| `media_spool_dir` | `$PREFIX/var/cache/wamux` |
| `database_url` | `sqlite://$PREFIX/var/lib/wamux/wamux.db` |

The daemon creates the store directory (mode `0700`, because the store holds
every account's keys) and the spool directory when they are missing. Override
any of them with `wamux.toml` or `WAMUX_*` as on any other platform, see
[`wamux.toml.example`](../wamux.toml.example).

A build for plain Android (an app that embeds the daemon, not the Termux
build) has no defaults for these three: its directories belong to that app. It
refuses to start until all three are set, and the error names each variable.

## Run it and connect

```sh
wamux
```

The socket is `$PREFIX/var/run/wamux.sock`. A client on the phone opens it
directly. A client on another machine can reach it by forwarding the socket
over ssh (`sshd` from the `openssh` package):

```sh
ssh -L /tmp/wamux.sock:/data/data/com.termux/files/usr/var/run/wamux.sock -p 8022 <phone>
grpcurl -unix -plaintext /tmp/wamux.sock list
```

The peer allowlist works: `SO_PEERCRED` is available in Termux, so
`allowed_uids` and `allowed_gids` (see
[Allowlist by peer credentials](DEPLOYMENT.md#allowlist-by-peer-credentials-77))
behave as on Linux. Termux's own uid is the one a local client has.

## Keep it running

Run it as a service, so it restarts and keeps a log:

```sh
pkg install termux-services    # then restart Termux once
sv-enable wamux                # starts now and at every Termux start
sv status wamux
```

The package ships the service disabled (a `down` file), so nothing starts until
you run `sv-enable`. Output goes to `$PREFIX/var/log/sv/wamux/`. Take a wake
lock so the CPU stays on with the screen off:

```sh
termux-wake-lock
```

## What still stops it

A service and a wake lock are not enough on a phone. Android kills or freezes a
background process in several ways, and each needs its own setting.

- **The phantom process killer (Android 12 and newer).** Android ends the
  child processes of an app past a limit of 32, and Termux's services count as
  children. On Android 14 and newer, turn on the developer option **Disable
  child process restrictions**. Before that, from a computer with `adb`:

  ```sh
  adb shell device_config set_sync_disabled_for_tests persistent
  adb shell device_config put activity_manager max_phantom_processes 2147483647
  ```

- **Battery optimization.** In the Android settings, set Termux's battery use
  to **Unrestricted** (no optimization). Otherwise Android stops it in the
  background even with a wake lock.
- **Wi-Fi with the screen off.** Many phones drop Wi-Fi when the screen turns
  off. Keep Wi-Fi on during sleep (the Wi-Fi advanced settings, or the
  vendor's "keep Wi-Fi on" option). A dropped connection reconnects by itself
  once the network is back, but nothing arrives while it is down.
- **The vendor's task killer.** Some skins (MIUI, ColorOS and others) have
  their own list of apps allowed to run in the background. Add Termux to it
  and lock it in the recents list.

## Troubleshooting

- `no default on this platform for ...`: a plain Android build with the
  variables unset. Set the named variables.
- The service does not start after a reboot: open Termux once. The service
  daemon starts with the app.
