# Releasing

1. Set the same version in `Cargo.toml` (`[workspace.package]`) and `src-tauri/tauri.conf.json`,
   run `cargo check` so `Cargo.lock` follows, and merge that through a pull request. `main` is
   protected: changes land by pull request once the **Lint and test** check passes, and it can't
   be force-pushed or deleted.
2. Tag the merged commit and push the tag:
   `git switch main && git pull && git tag -a v0.2.0 -m "ViuMic 0.2.0" && git push origin v0.2.0`.
   The **Release** workflow builds
   the installer, the signed update files (`latest.json` and `.sig`) and `SHA256SUMS.txt` into a
   draft release. It stops early if the tag and the two versions don't match.
3. Check the draft (try the installer), then publish it. Installed copies look for updates on
   start and every 6 hours, and show **Update ready**; nothing installs until the user clicks.
   Drafts and prereleases never reach them.

A failed run can be re-run from the Actions tab: it reuses the tag and the draft.

## Update signing

Updates are checked against the public key in `src-tauri/tauri.conf.json`
(`plugins.updater.pubkey`). The matching private key is the repository secret
`TAURI_SIGNING_PRIVATE_KEY` (generated with `pnpm tauri signer generate`, no password). Keep an
offline copy of it: without it, installed copies can never update again, and a new key means
everyone reinstalls by hand.

This is Tauri's free update signature, not Windows code signing. The installer itself is
unsigned, so Windows SmartScreen asks once (**More info → Run anyway**).

A signed build on your own PC:

```
TAURI_SIGNING_PRIVATE_KEY=path/to/viumic-updater.key pnpm tauri build --config src-tauri/tauri.release.conf.json
```

Plain `pnpm build` skips the update files and needs no key.

## README images

`docs/images/src` has the templates for `banner.png` and `how-it-works.png`; the render
command is at the top of each file. `app.png` and `app-wide.png` are screenshots of the
installed app.
