# Linux release procedure

The first targets are Linux x86_64 AppImage, installation from source, and Arch/AUR.
Use the portable container build for distributables; a developer Arch build can
require a newer glibc and is suitable only for that machine's development checks.

1. Review the confirmed OCR redistribution permission in `docs/module-provenance.md`,
   dependency notices/declarations, and GPL corresponding-source directions.
   Remove the unpublished oversized speech-model Git blob through the agreed
   history cleanup before pushing the release commit; checksum-pinned model
   bootstrapping supplies those files for fresh source builds.
2. Run the frontend/Rust suites and isolated live diagnostics, including documents,
   OCR, code retrieval, and snapshot restoration. Keep their results with the candidate.
3. Run `npm run release:linux`. It builds on Debian 12/glibc 2.36 with Rust 1.88
   and Node 22.15, tests the installed renderer, and creates AppImage, a Linux
   payload archive, native-library manifest, and `SHA256SUMS` in `release/github`.
   To preserve an earlier candidate while qualifying a changed working tree, set
   `AYLA_RELEASE_OUTPUT="$PWD/release/followup-2026-10-14"` for that command.
   Input hashes in `source-manifest.json` identify the copied working tree;
   a base commit alone does not identify uncommitted changes.
4. Verify actual downloaded artifacts on Arch and a supported non-Arch machine,
   with X11/Wayland where available. Complete microphone, real speech, research
   quality, and mixed-workload soak checks in the production readiness plan.
5. Generate the Arch binary recipe with `node scripts/update-aur.mjs` and build
   source/binary recipes in a clean Arch chroot. The source recipe uses system
   Electron 41; the AppImage and binary recipe bundle the tested runtime.
6. Review and commit the release changes, then create a signed immutable `v0.1.0`
   tag. The source installer verifies tags unless the user explicitly supplies
   an immutable commit or requests the development `--allow-unsigned` override.
7. Sign `SHA256SUMS` with the release key, for example `gpg --armor --detach-sign
   release/github/SHA256SUMS`. Publish the public key/fingerprint separately so
   users can verify it. Release signing is a maintainer action; no key is generated
   or assumed by the build scripts.
8. Upload the reviewed artifacts and signatures to the GitHub release. Submit
   AUR recipes/metadata separately after checking name availability. No script
   automatically publishes to GitHub or AUR.

Application rollback preserves user data but does not reverse a database schema
migration. Create and verify a knowledge snapshot before upgrading schemas.
Snapshots contain private knowledge; keep them local and do not attach them to
public support reports.

For source installation, clone the tagged repository and run `./install.sh
--source-dir "$PWD"`. Or run `./install.sh --ref v0.1.0` with the signing public
key imported into the user's trust store. Source prerequisites are Node >=22.12,
Rust >=1.88, Git, a C/C++ toolchain, pkg-config, and development libraries for
OpenSSL, xz, and bzip2. Docker is needed only for the portable build and optional
service stack. Ollama models are set up explicitly with `npm run models:setup`.
