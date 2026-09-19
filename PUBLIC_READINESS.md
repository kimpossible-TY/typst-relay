# Publication review — Tinymist Flow

Reviewed on 2026-09-19. **Suitable for publication as an experimental source fork, with the limits below. Visibility remains private.** This is not a production-readiness certification or an exhaustive audit of inherited upstream history.

## License and identity

The upstream project is Apache-2.0. Its [license](LICENSE) is retained unchanged, as is the inherited `editors/neovim/NOTICE`. The README identifies Myriad-Dreamin and the Tinymist contributors, labels Flow an independent derivative, and does not reuse upstream CI badges as evidence for this fork. Modified upstream source and tooling carry change notices. New fork documentation and tests are distributed under the repository license.

Apache-2.0 permits derivative redistribution subject to preserving the license, applicable notices and attribution, and identifying modified files. It does not grant upstream trademark rights. See [Apache-2.0 sections 4 and 6](https://www.apache.org/licenses/LICENSE-2.0). Retain these notices when distributing source; review bundled dependency notices separately before publishing binary releases.

## What was checked

- The private repository is independent (`isFork: false`). Its published refs contain one development branch and the first-patch preservation tag.
- The two fork commits following upstream base `32f908199ee17ea295512bbc27166e890c438175` were reviewed, together with the branding changes. The fork-added source is a dependency-discovery change, regression fixtures/harness, and development records.
- A targeted scan of those two commits found no GitHub-token, AWS access-key, private-key-header, VS Code connection-token, absolute macOS home-path, or private document-directory patterns. This is limited pattern screening, not proof that arbitrary secrets cannot exist.
- No `.local/` or `.worktrees/` files are tracked. Private document copies, raw editor logs, settings backups, benchmark artifacts, and binaries were excluded. Root ignore rules now preserve these exclusions in future clones.
- Fork commit author addresses use GitHub's noreply address. Deployment notes still contain platform, timestamps, process IDs, relative backup locations, and binary hashes. These are operational metadata, not credentials; they would become public with the history.
- GitHub Actions is disabled. The repository has zero workflow runs, releases, and issues at review time. Inherited workflows remain present but must be reviewed before enabling Actions or publishing artifacts.
- The original upstream Git history is retained. It was already publicly available; a complete secret or third-party-content audit of that inherited history was not performed.

Deleting a file in a later commit does not remove earlier copies. Publication exposes the retained history as well as the current files. GitHub also makes Actions history and logs public on a visibility change; see [GitHub's visibility documentation](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/managing-repository-settings/setting-repository-visibility).

## Release scope

Publishing this source as **experimental** is reasonable on the evidence reviewed. Publishing a stable release would overstate the current validation:

- Extended editing sessions, memory stability, and remote-preview delivery still need measurement.
- Initial deployment was macOS arm64 with the Tinymist 0.15.8 extension. Other platforms and newer extension versions are not established compatibility claims.
- Timings are individual requests, including an intentionally expensive reproducer, not a representative performance distribution.
- Existing upstream version metadata remains; a future distributed binary needs clear fork provenance and its own artifact identification.
- Previous scoped tests passed; no claim is made that the entire inherited CI suite passes. See the [validation record](openspec/changes/static-import-discovery/validation.md).

No change to visibility was made during this review. A later publication step should recheck changes and uploaded artifacts since this review, then explicitly change this repository to public. Keep Actions disabled until its inherited release and publishing configuration has been adapted.
