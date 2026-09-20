# Release checklist

What has to be true before a tag, and what to do after it. It exists because of 0.4.1-0.4.3: every
automated gate passed and every installer produced a window with nothing in it, and the reason was that
nobody had opened the *installed* app. The checks below are the ones that would have caught it.

## Before the tag

Run from `sdc/`. All of these run in CI as well; running them here is what makes a failed CI run
surprising rather than expected.

| Step | Command | What it proves |
| --- | --- | --- |
| Types | `pnpm typecheck` | The frontend and the build tooling type check. |
| Lint | `pnpm lint` | `eslint`, including the hooks rules. |
| Frontend tests | `pnpm test` | The reducer, the command registry, and the store-selector rule. |
| The window renders | `pnpm build && pnpm --filter @sdc/app smoke` | The built bundle mounts in a real browser: `#root` is not empty, `#app` is there, there is text, and nothing threw. |
| Daemon tests | `cargo test --manifest-path sdcd/Cargo.toml` | 101 tests, including the three that start the real daemon (idle exit, `host.shutdown`, hand-started stays). |
| Clippy | `cargo clippy --manifest-path sdcd/Cargo.toml --all-targets -- -D warnings` and the same for `app/src-tauri/Cargo.toml` | No warnings, in the daemon or the bridge. |
| Versions agree | `grep -r '"version"' package.json app/package.json app/src-tauri/tauri.conf.json app/src-tauri/Cargo.toml sdcd/Cargo.toml` | One number everywhere, plus `strings.ts`'s three copies. |

## The packaged app (Windows, WebView2)

The one check that is not automated, and the one that matters most: an installer that produces a
working app. On Windows the whole sequence is a few minutes.

```powershell
# 1. Build, then install silently into %LOCALAPPDATA%\SDC.
cd sdc; pnpm tauri:build
$setup = 'app/src-tauri/target/release/bundle/nsis/SDC_<version>_x64-setup.exe'
Start-Process $setup -ArgumentList '/S' -Wait

# 2. Open it with the WebView2's debugging port, so the DOM can be read rather than guessed at.
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9227'
Start-Process "$env:LOCALAPPDATA\SDC\sdc.exe"
node H:/SDC/_verify/boot-check.mjs 9227 http://tauri.localhost   # expect: #app present, grid, no errors
node H:/SDC/_verify/shot.mjs 9227 H:/SDC/_verify/installed.png   # a screenshot, to look at

# 3. The daemon is this build's, and has no console window of its own.
Get-Process sdcd | Select-Object Path, MainWindowTitle            # the installed sdcd.exe, empty title
#    and `host.status` reports the same version as the installer.

# 4. Quitting takes the daemon with it; killing the app leaves it to its --idle-exit.
(Get-Process sdc).CloseMainWindow(); Start-Sleep 4; Get-Process sdc, sdcd   # both gone
Start-Process "$env:LOCALAPPDATA\SDC\sdc.exe"; Start-Sleep 7
Get-Process sdc | Stop-Process -Force; Start-Sleep 12; Get-Process sdcd     # gone within the grace

# 5. Uninstall, so the next test starts from nothing.
Start-Process "$env:LOCALAPPDATA\SDC\uninstall.exe" -ArgumentList '/S' -Wait
```

`_verify/` is a developer-only directory (gitignored): `boot-check.mjs` reads `#root`'s children, the
computed display of `#app` and the console log over CDP; `shot.mjs` writes a PNG. Both take a port and,
for `boot-check.mjs`, the window's URL.

## After the tag

1. Push the tag and watch the four `release` jobs plus `Secret scan`:
   `node H:/SDC/_verify/ci-status.mjs` (or the Actions tab).
2. All four jobs must be green **before** the release is announced: a macOS or Linux job that failed
   leaves the release with a subset of the assets, which is worse than no release.
3. Check the asset list: `*_x64-setup.exe`, `*_x64_en-US.msi`, `*_x64.dmg`, `*_aarch64.dmg`,
   `*_amd64.AppImage`, `*_amd64.deb` - six files for one version.
4. If a job failed: fix, commit, then move the tag (`git tag -d vX.Y.Z && git push origin :vX.Y.Z`,
   re-tag the new commit and push). The existing release is reused and the missing assets are added;
   do not leave a tag pointing at a commit whose macOS build fails.
