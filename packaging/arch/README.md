# Arch and AUR delivery

`ayla-ai/PKGBUILD` builds the application from its release tag and uses system
Electron 41. Native archives have fixed SHA-256 checksums; dependencies are
acquired in `prepare()`, and compilation/testing are offline.

Build in a clean Arch chroot and inspect with `namcap` before submitting. Generate
`.SRCINFO` with `makepkg --printsrcinfo`. The source recipe needs the corresponding
GitHub release tag to exist. Tag the reviewed release commit, preferably with a
trusted signing key; do not move release tags after publishing them.

The `-bin` recipe is generated from the official Linux payload and its actual
checksum by `scripts/update-aur.mjs` after the portable release build. It bundles
the verified Electron runtime and conflicts with the source package. Neither
recipe starts Docker/Ollama or downloads models from installation hooks.

AUR publication is performed separately by the maintainer. Package names must
be checked for availability and metadata reviewed before upload.

References: [Electron package guidelines](https://wiki.archlinux.org/title/Electron_package_guidelines),
[clean chroot builds](https://wiki.archlinux.org/title/DeveloperWiki:Building_in_a_clean_chroot).
