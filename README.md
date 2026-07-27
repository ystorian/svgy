# SVGY

Turn one SVG into all the icons a project needs: resized and optimized SVGs, PNGs, Windows `.ico` macOS `.icns`, or all of them in a single pass.

## Install

```shell
cargo install --path .
```

## Synopsis

```shell
svgy <input.svg> [targets] [options]
```

With one input file, preferrably a square SVG, **svgy** can generate many icons formats in one pass.

## Defaults

The command `svgy file.svg` without parameter:

- Resizes the SVG so its longest side is `1024`, by rewriting the `viewBox`.
- Saves the SVG with a default suffix as `file.svgy.svg`.

This is equivalent to:

```shell
svgy file.svg --svg --suffix=svgy --size=1024
```

- `--svg`: Create an SVG.
- `--suffix=svgy`: Add the suffix `svgy` to the output file name.
- `--size=1024`: Resize so the longest side is 1024.

Asking for any target turns the implicit `--svg` off. Add `--svg` explicitly to keep it.

## Examples

Create two icons for macOS and Windows in their respective subfolders, and a tiny SVG for the
favicon in the same directory:

```shell
svgy example.svg --icns=macos/example.icns --ico=windows/example.ico --svg=favicon.svg --size=32
```

> - **`example.svg`**
> - `macos`
>   - `example.icns` _16x16 to 1024x1024_
> - `windows`
>   - `example.ico` _16x16 to 256x256_
> - `favicon.svg` _32x32_

_Note: `--size` never applies to icons, they keep their fixed size sets (see [Sizing](#sizing))._

Create a PNG and keep the resized SVG alongside it:

```shell
svgy example.svg --png --svg
```

> - **`example.svg`**
> - `example.svgy.svg` _1024x1024, **--svg** always uses the `svgy` suffix to prevent overwriting the source SVG._
> - `example.svgy.png` _1024x1024, default size_


Fit the artwork inside a circle, for a round contact avatar:

```shell
svgy example.svg --round
```

> - **`example.svg`**
> - `example.round.svg` _1024x1024, **--round** uses the `round` suffix._


## Output targets


- `--svg[=<path>]`: Convert to an optimized and minified SVG.
- `--png[=<path>]`: Convert to an optimized PNG.
- `--ico[=<path>]`: Convert to an optimized Windows icon `.ico`.
- `--icns[=<path>]`: Convert to an optimized macOS icon `.icns`.
- `--round[=<path>]`: Convert to an SVG whose artwork is centered and fitted inside a circle.

The output images are by default resized to fit 1024 x 1024, except icons which have specific sets
of sizes per platform.

## Sizing

All sizing parameters are optional.

- `--size=<pixels>`: Resize to a square of `<pixels>` by `<pixels>`, preserving the aspect ratio.
- `--width=<pixels>`: Resize to the specified width, keeping proportions.
- `--height=<pixels>`: Resize to the specified height, keeping proportions.
  Passing `--width` and `--height` together fits the artwork inside that box.
- `--no-resize`: Keep the original dimensions.

The sizing parameters `--size` | `--width` | `--height` are mutually exclusive.

For icons, non-square source SVGs are resized and centered. `.ico` and `.icns` have fixed size sets,
so sizing parameters do not apply to them and are ignored when an icon target appears alongside a
sizeable one.

## Rounding

- `--padding[=<0.0..1.0>]`: Fraction of the inscribed circle's radius to leave empty around the
  artwork. `0.0` by default, or `0.1` when the source has a full-canvas background; bare
  `--padding` means `0.1`. svgy prints which default it applied. The upper bound is exclusive.

`--padding` applies to `--round` only.

## Output naming

- If the destination is not specified, the output is written beside the source as
  `<name>.<suffix>.<extension>`, with `svgy` as the default suffix.
  - `svgy example.svg --png` -> `example.svgy.png`
  - `svgy example.svg --icns` -> `example.svgy.icns`
- `--suffix=<suffix>`: Use `<suffix>` instead of `svgy`.
  - `svgy example.svg --png --suffix=v2` -> `example.v2.png`
- `--round` is the one exception: its default suffix is `round`, so that `--svg` and `--round` can
  be asked for together without both claiming `example.svgy.svg`.
  - `svgy example.svg --svg --round` -> `example.svgy.svg` and `example.round.svg`

## Behaviour

- **Order of operations**: read -> resize -> round -> write each target. Rounding runs last, inside
  whatever canvas `--size` set, and the raster targets are rendered from the resized SVG.
- **`--round` is a target, not a modifier.** `svgy logo.svg --round --icns` writes a round SVG _and
  a normal `.icns`_ — the icon is not round-fitted. For round icons, run svgy twice and feed it the
  round SVG.
- **One input at a time.** Globs and multiple inputs are not supported; use a shell loop.
- **Missing directories are created.** A destination may name a subdirectory that does not exist
  yet, as the icons example above does.
- **Existing files are overwritten** without asking.
- **Exit codes**: `0` on success, `1` on any error, with a message on stderr.

## Other options

- `--no-optimize`: Skip PNG optimization. Optimization dominates the runtime of an `.icns` or
  `.ico`, so this is the flag to reach for when iterating.
- `--no-legacy-ico`: Skip the 256-color entries in the Windows icon, keeping only the PNG ones.
  Saves about 3.6 KiB, at the cost of dropping support for 256-color sessions and pre-Vista shells.

## Planned

Not implemented yet. Documented here so the intended surface is on record.

### Targets

- `--all-app[=<app>]`: Convert to all native app icons, using the source SVG location as the root
  directory.
  - Main icon: `app/<app>.svg`
  - Windows: `windows/<app>.ico`
  - macOS: `macos/<app>.icns`
  - Linux: `linux/<app>_size.png` and `linux/<app>.svg`
- `--svgz[=<path>]`: Convert to an optimized and GZIP-compressed SVGZ.
- `--avif[=<path>]`: Convert to an optimized lossless AVIF.
- `--liquid[=<icon>]`: Convert to an optimized macOS Liquid Glass icon `.icon` directory.
- `--linux[=<app>]`: Convert to a set of optimized PNGs (`<size>.png`) and an SVG (`scalable.svg`)in the `app` directory.

### Actions

- `--set-folder-icon[=<dir>]`: Set the folder icon, macOS only. Without a value, the icon is for the directory containing the source SVG.

### Parameters

- `--in-place`: Overwrite the source `.svg`. SVG output only. Warning: this is a destructive
  parameter.
- `--keep-ids`: Keep SVG IDs, for example `<path id="this-is-kept">`. Depends on SVG optimization,
  below, since nothing strips IDs until that ships.
- `--round-anchor=<shape|canvas>`: Whether `--round` recenters the artwork on the canvas (`canvas`,
  the default and current behaviour) or scales it in place, leaving its center where it is
  (`shape`).
- `--if-exists=<replace|keep|if-smaller|suffix>`: What to do when the destination exists. `replace`
  by default. `keep` to leave existing files alone, `if-smaller` to replace only when the new file
  is smaller, `suffix` to keep existing files and write to the suffixed name instead, replacing an
  existing suffixed file.
- `--quiet`: No output. Mutually exclusive with `--verbose`.
- `--verbose`: Print out all operations. The default prints one line per file written.
- `--all-app-dir=<dir>`: See `--all-app`.
- `--app_id=<app_id>`: See `--all-app`.

### Optimization

SVG optimization. Until it ships, an SVG output is resized but not optimized. When it lands,
`--no-optimize` will skip it too, so the flag keeps its meaning.

IDs referenced from the document (`url(#gradient)`, `href="#clip"`, and the like) will never be
stripped, whether or not `--keep-ids` is passed; the flag governs the unreferenced ones.

## References

### Sizes

| Type                                       | Extension | Icons in set | Size                   |
| ------------------------------------------ | --------- | :----------: | ---------------------- |
| Scalable Vector Graphics                   | `.svg`    |     _1_      | 1024 x 1024            |
| Compressed Scalable Vector Graphics (gzip) | `.svgz`   |     _1_      | 1024 x 1024            |
| Portable Network Graphics                  | `.png`    |     _1_      | 1024 x 1024            |
| [macOS icon](#macos-icon)                  | `.icns`   |      11      | 16 x 16 to 1024 x 1024 |
| [Windows icon](#windows-icon)              | `.ico`    |      8       | 16 x 16 to 256 x 256   |
| AV1 Image File Format                      | `.avif`   |     _1_      | 1024 x 1024            |

`.icns` and `.ico` are multi-resolution containers: they hold every entry listed below, and the
OS picks one. Their size sets are fixed and cannot be overridden, on purpose: the default is
meant to be the good, opinionated answer.

### macOS icon

File extension: `.icns`

| Size         | Pixels      | Type   |  Format  |
| ------------ | ----------- | ------ | :------: |
| 16 x 16      | 16 x 16     | `ic04` | ARGB-RLE |
| 16 x 16 @2   | 32 x 32     | `ic11` |   PNG    |
| 32 x 32      | 32 x 32     | `ic05` | ARGB-RLE |
| 32 x 32 @2   | 64 x 64     | `ic12` |   PNG    |
| 48 x 48      | 48 x 48     | `icp6` |   PNG    |
| 128 x 128    | 128 x 128   | `ic07` |   PNG    |
| 128 x 128 @2 | 256 x 256   | `ic13` |   PNG    |
| 256 x 256    | 256 x 256   | `ic08` |   PNG    |
| 256 x 256 @2 | 512 x 512   | `ic14` |   PNG    |
| 512 x 512    | 512 x 512   | `ic09` |   PNG    |
| 512 x 512 @2 | 1024 x 1024 | `ic10` |   PNG    |

> **Notes:**
>
> - The 16 x 16 and 32 x 32 entries store straight-alpha ARGB with each channel PackBits-RLE encoded; every larger entry embeds its PNG bytes verbatim.
> - Eleven entries cover eight distinct pixel sizes, and each size is rendered once. The 256 and 512 renders are also encoded once and their PNG bytes reused across both OSTypes. The 32 render is encoded twice, because its two entries want different formats: ARGB-RLE for `ic05` and PNG for `ic11`.

### Windows icon

File extension: `.ico`

Eight entries for six sizes: the two smallest are stored twice, once in full color and once for
256-color consumers. Pass `--no-legacy-ico` for the six PNG entries alone.

| Size      | Format | Declared depth | Alpha      |
| --------- | :----: | :------------: | ---------- |
| 16 x 16   |  PNG   |     32 bpp     | 8-bit      |
| 16 x 16   |  BMP   |     8 bpp      | 1-bit mask |
| 32 x 32   |  PNG   |     32 bpp     | 8-bit      |
| 32 x 32   |  BMP   |     8 bpp      | 1-bit mask |
| 48 x 48   |  PNG   |     32 bpp     | 8-bit      |
| 64 x 64   |  PNG   |     32 bpp     | 8-bit      |
| 128 x 128 |  PNG   |     32 bpp     | 8-bit      |
| 256 x 256 |  PNG   |     32 bpp     | 8-bit      |

> **Notes:**
>
> - 256 x 256 is the largest size an `.ico` can address, as the width and height fields are single bytes (0 meaning 256).
> - PNG entries are optimized by oxipng. Their declared depth is what Windows compares against the display when choosing between two entries of the same size, so it stays at the conventional 32 even where oxipng has reduced the PNG to an 8-bit palette; readers take the real depth from the IHDR.
> - The BMP entries are the classic bottom-up 8bpp DIB: a 256-entry color table, then the color data, then the 1-bit AND mask. Colors come from a median-cut palette of the source render.
> - They exist for sessions capped at 256 colors, such as an old `mstsc` or the "Limit maximum color depth" policy, where Windows prefers an entry declaring 8 bpp. They also keep those two sizes readable by pre-Vista shells, which cannot decode PNG entries at all.
> - They cost about 3.6 KiB: the color table alone is a fixed 1 KiB per entry, written in full rather than trimmed with `biClrUsed`, since the point of these entries is to be boring for old readers. `--no-legacy-ico` drops them.
> - A 1-bit mask has no partial transparency. Pixels below 50% alpha vanish and the rest become fully opaque, so anti-aliased edges turn hard. This only affects the 256-color entries.

### Linux icons

See:

- [Icon Theme Specification](https://specifications.freedesktop.org/icon-theme/latest/)
- APPImage: [The filesystem image](https://github.com/AppImage/AppImageSpec/blob/master/draft.md#the-filesystem-image)


Installation path:

- SVG:
  - `/usr/share/icons/hicolor/scalable/apps/<id.app>.svg` _e.g. com.ystorian.svgy.svg_
- PNG:
  - `/usr/share/icons/hicolor/256x256/apps/<id.app>.png` _e.g. com.ystorian.svgy.png_


### Pipelines

- SVG: rewrite the root `viewBox` -> multiply the scale into every coordinate in place -> round
  coordinates to 6 decimals
- ROUND: measure the artwork's minimum enclosing circle (background fills excluded) -> bake the
  scale and translation that map it onto the inscribed circle into every coordinate
- PNG: render with resvg -> optimize with oxipng
- ICO: render each size with resvg -> encode PNG -> optimize with oxipng -> embed verbatim, plus a
  median-cut 256-color BMP for 16 x 16 and 32 x 32
- ICNS: render each size with resvg -> 16 and 32 to straight-alpha ARGB with PackBits RLE, larger
  sizes to PNG -> optimize with oxipng -> embed verbatim

Resizing adds no `transform`: a uniform scale is origin-independent and commutes with rotation, so
multiplying it into each coordinate renders identically to the source while preserving the original
element structure. Existing transforms are rewritten in place rather than flattened.

Every PNG goes through oxipng at preset 6. Icons are written once and read forever, so the pipeline
trades time for bytes, but only where the trade is worth taking: up to 512 x 512 the Zopfli deflater
saves roughly 2 to 5% over libdeflater, and above that its runtime grows far faster than the saving,
so the largest entries keep preset 6's libdeflater. Capping Zopfli at 512 cuts a full `.icns` to
roughly a quarter of the time and costs 1.8% in size. Pass `--no-optimize` to skip optimization
entirely.

## Requirements

Rust, edition 2024.

`--set-folder-icon` (planned) will be OS-specific. Everything else is cross-platform: an `.icns` can
be produced on Windows and an `.ico` on macOS.

## Licence

Dual-licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you state otherwise, any contribution intentionally submitted for inclusion in svgy, as
defined in the Apache-2.0 licence, shall be dual-licensed as above, without any additional terms or
conditions.
