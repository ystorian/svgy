# <img src="https://raw.githubusercontent.com/ystorian/svgy/main/svgy.svg" width="48" align="absmiddle" alt="Svgy logo"> Svgy

**Generate icons from SVG**

![Build Status](https://github.com/ystorian/svgy/actions/workflows/ci-rust.yaml/badge.svg)
[![Crates.io](https://img.shields.io/crates/v/svgy.svg)](https://crates.io/crates/svgy)
[![docs.rs](https://docs.rs/svgy/badge.svg)](https://docs.rs/svgy)

Make all the icons of a project from one SVG, in one pass: optimized `.svg`, `.png`, Windows `.ico`,
and macOS `.icns`.

## Install

Install a signed binary for Linux, macOS, or Windows, on x64 or arm64:

```shell
cargo binstall svgy
```

Or build from [crates.io](https://crates.io/crates/svgy):

```shell
cargo install svgy
```


## Usage

```shell
svgy <input.svg> [targets] [options]
```

Use a square SVG for the best result.

### Defaults

`svgy file.svg` makes `file.svgy.svg`.

Svgy:

- Makes the image fit in 1024 x 1024.
- Puts the artwork in the center of a square.
- Makes the SVG smaller with `oxvg`.
- Optimizes further while keeping visual changes below 2%.

This is the same as:

```shell
svgy file.svg --svg --suffix=svgy --size=1024 --precision=0.02
```

### Examples

Make a macOS icon, a Windows icon, and a small SVG favicon:

```shell
svgy example.svg --icns=macos/example.icns --ico=windows/example.ico --svg=favicon.svg --size=32
```

> - **`example.svg`**
> - `macos`
>   - `example.icns` _16x16 to 1024x1024_
> - `windows`
>   - `example.ico` _16x16 to 256x256_
> - `favicon.svg` _32x32_

`--size` has no effect on icons since they always have the same sizes.

Make a PNG and an SVG:

```shell
svgy example.svg --png --svg
```

> - **`example.svg`**
> - `example.svgy.svg` _1024x1024_
> - `example.svgy.png` _1024x1024_

Put the artwork in a circle, for a round avatar:

```shell
svgy example.svg --round
```

> - **`example.svg`**
> - `example.round.svg` _1024x1024_

### Main options

- `--svg`, `--png`, `--ico`, `--icns`, `--round`: Select the files to make, add `=<path>` for the
  destination.
- `--size=<pixels>`: Make the image fit in a square. The default is `1024`.
- `--suffix=<suffix>`: Change the end of the file name. The default is `svgy`.
- `--precision=<fraction>`: Set the maximum change to the SVG image. The default is `0.02`.
- `--no-optimize`: Do not make the files smaller. Svgy is then much faster.

Refer to [Options](docs/options.md) for all options.

## Documentation

- [Options](docs/options.md): All targets and options.
- [Behavior](docs/behavior.md): Steps, text, images, files, and exit codes.
- [Formats](docs/formats.md): The images in the `.icns` and `.ico` files.
- [Optimization](docs/optimization.md): How Svgy makes files smaller.
- [Roadmap](docs/roadmap.md): Planned features.

## Requirements

Rust, edition 2024.

## License

Dual-licensed under either of

- [Apache License](LICENSE-APACHE), Version 2.0
- [MIT License](LICENSE-MIT)

at your option.

Unless you state otherwise, any contribution intentionally submitted for inclusion in `svgy`, as
defined in the Apache-2.0 license, shall be dual-licensed as above, without any additional terms or
conditions.
