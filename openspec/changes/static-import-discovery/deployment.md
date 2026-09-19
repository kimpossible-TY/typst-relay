# Private repository and first deployment

Date: 2026-09-19 (Asia/Seoul).

## Source preservation

- Private independent repository: https://github.com/kimpossible-TY/tinymist-flow
- Patch commit: `288b215d190b3dad69dd951cb391da6402a852d1`.
- Branch: `perf/static-import-discovery`.
- Preservation tag: `fork-v0.15.8-p1`.
- `origin` points to the private repository; `upstream` still points to Myriad-Dreamin/tinymist, and the initial public fork remains as `public-fork`.
- GitHub Actions is disabled in the new private repository, so uploading inherited upstream workflows does not run builds or release jobs.

## Installed artifact

The previously tested binary is installed at:

```text
~/.local/share/tinymist-fork/versions/288b215d/tinymist
```

SHA-256:

```text
62fb0d190399f9a96d75b504f4711f9dc651722fa7b39330a9fc8d04e6cfaf21
```

The binary was built before the preservation commit and retains upstream 0.15.8 version metadata. The manifest and checksum identify the patched artifact; version text alone does not distinguish it from upstream.

## Active editor configuration

The remote VS Code extension host is running on the same macOS arm64 machine. The Typst workspace's `.vscode/settings.json` now enables semantic tokens and sets `tinymist.serverPath` to the installed artifact's absolute path. Existing root/entry settings and the effective on-save preview refresh setting are preserved.

The active language client cached its executable path. To switch the running session without restarting unrelated extensions, the bundled executable of `myriad-dreamin.tinymist-0.15.8-darwin-arm64` was backed up and atomically replaced with the identical patched binary. The old Tinymist process alone was terminated, and VS Code automatically restarted it. Future extension activation can load the explicit `serverPath`.

Verified at deployment:

- New Tinymist PID: `78467`, parent remote extension host: `7058`.
- The running executable's inode matches the patched bundled executable; its checksum matches the retained and installed artifact.
- Server initialized and reopened the main entry and the active chapter buffer.
- At `2026-09-19T08:16:27Z`, the server applied `semanticTokens: enable`, the new `serverPath`, and preview refresh `onSave`; full language-server mode remains enabled.
- No `stall detected` was observed in the initial post-restart log window. Existing field-access import warnings and an unhandled `$/setTrace` notification remain; this is not a claim that all warnings are fixed.

Preview panels were not reopened by this deployment, and long-duration editing, memory stability, and remote preview delivery still need real-session validation.

## Local backups and rollback

Ignored deployment records and backups are under `.local/deployment/20260919T171548/` in this checkout. `.local/deployment/latest.json` records exact paths, checksums, the running PID at verification, and the log location.

To roll back, restore the backed-up bundled executable `tinymist.bundled.before`, remove the workspace's `tinymist.serverPath` override, and return `tinymist.semanticTokens` to its previous value (`disable`). Then restart Tinymist. When restoring settings, preserve unrelated settings changed since deployment; `settings.before.json` is the original reference. Do not restore only the bundled executable while retaining the custom serverPath override, because a subsequent activation would still select the patched binary.

Private document content, raw editor logs, binaries, and local backup files were not added to the source repository.
