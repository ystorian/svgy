# Roadmap

These features are not available yet.

## Targets

- `--all-app[=<app>]`: Make all app icons. Svgy puts them in folders next to the input.
  - Main icon: `app/<app>.svg`
  - Windows: `windows/<app>.ico`
  - macOS: `macos/<app>.icns`
  - Linux: `linux/<app>_<size>.png` and `linux/<app>.svg`
- `--svgz[=<path>]`: Make a small SVG, compressed with gzip (`.svgz`).
- `--avif[=<path>]`: Make an AVIF image with no quality loss.
- `--liquid[=<icon>]`: Make a macOS Liquid Glass icon (an `.icon` folder).
- `--linux[=<app>]`: Make PNG files (`<size>.png`) and an SVG (`scalable.svg`) in the `<app>`
  folder.

## Actions

- `--set-folder-icon[=<dir>]`: Set the icon of a folder. macOS only.
  - With no value, Svgy uses the folder of the input.

## Options

- `--in-place`: Write over the input `.svg`. SVG only.
  - **Warning:** You lose the original file.
- `--keep-ids`: Keep all SVG IDs, such as `<path id="this-is-kept">`.
  - Today, Svgy makes IDs shorter and removes the IDs that nothing uses.
  - Svgy never removes an ID that the SVG uses, such as `url(#gradient)` or `href="#clip"`.
- `--round-anchor=<shape|canvas>`: Select where `--round` puts the artwork.
  - `canvas`: In the center of the canvas. This is the default, and what Svgy does today.
  - `shape`: At the same place. Svgy only scales it.
- `--if-exists=<replace|keep|if-smaller|suffix>`: Select what to do when the file exists.
  - `replace`: Write over the file. This is the default.
  - `keep`: Do not change the file.
  - `if-smaller`: Write over the file only if the new file is smaller.
  - `suffix`: Keep the file. Write to a name with the suffix. Write over a file with that name.
- `--quiet`: Show nothing. You cannot use it with `--verbose`.
- `--verbose`: Show all steps. Today, Svgy shows one line for each file.
- `--all-app-dir=<dir>`: Refer to `--all-app`.
- `--app_id=<app_id>`: Refer to `--all-app`.

## Formats

| Type                                | Extension | Images in file | Size        |
| ----------------------------------- | --------- | :------------: | ----------- |
| Compressed Scalable Vector Graphics | `.svgz`   |      _1_       | 1024 x 1024 |
| AV1 Image File Format               | `.avif`   |      _1_       | 1024 x 1024 |
