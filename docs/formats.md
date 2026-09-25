# Formats

## Sizes

| Type                          | Extension | Images in file | Size                   |
| ----------------------------- | --------- | :------------: | ---------------------- |
| Scalable Vector Graphics      | `.svg`    |      _1_       | 1024 x 1024            |
| Portable Network Graphics     | `.png`    |      _1_       | 1024 x 1024            |
| [macOS icon](#macos-icon)     | `.icns`   |       11       | 16 x 16 to 1024 x 1024 |
| [Windows icon](#windows-icon) | `.ico`    |       8        | 16 x 16 to 256 x 256   |

An icon file holds many images of different sizes. The operating system selects the best one.

You cannot change the sizes in an icon. Svgy always uses the recommended sizes.

## macOS icon

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

**Notes:**

- `@2` is for high-resolution (Retina) screens. The image has two times more pixels.
- The two smallest images use ARGB-RLE. This is raw pixels with simple compression.
- All other images are PNG.
- The file has 11 images, but only 8 different pixel sizes. Svgy draws each size one time.

**Technical details:**

- ARGB-RLE stores straight alpha. Svgy compresses each channel with PackBits.
- Svgy encodes the 256 and 512 images one time, and uses them in two entries.
- Svgy encodes the 32 image two times: ARGB-RLE for `ic05`, and PNG for `ic11`.

## Windows icon

File extension: `.ico`

| Size      | Format | Declared depth | Transparency |
| --------- | :----: | :------------: | ------------ |
| 16 x 16   |  PNG   |     32 bpp     | 8-bit        |
| 16 x 16   |  BMP   |     8 bpp      | 1-bit mask   |
| 32 x 32   |  PNG   |     32 bpp     | 8-bit        |
| 32 x 32   |  BMP   |     8 bpp      | 1-bit mask   |
| 48 x 48   |  PNG   |     32 bpp     | 8-bit        |
| 64 x 64   |  PNG   |     32 bpp     | 8-bit        |
| 128 x 128 |  PNG   |     32 bpp     | 8-bit        |
| 256 x 256 |  PNG   |     32 bpp     | 8-bit        |

The file has 8 images for 6 sizes. The two smallest sizes are in the file two times:

- One PNG image with all colors.
- One BMP image with 256 colors.

Use `--no-legacy-ico` to remove the BMP images.

### PNG images

- 256 x 256 is the largest size that an `.ico` file can hold.
- Svgy makes the PNG images smaller with `oxipng`.
- The declared depth is always 32 bpp (bits per pixel). Windows uses this value to select an image.
  The real depth in the PNG can be smaller.

### BMP images

The BMP images are for old systems:

- Windows sessions with only 256 colors, such as some old remote desktops.
- Windows versions older than Vista. These cannot read PNG images in an icon.

Facts about the BMP images:

- They add about 3.6 KiB to the file.
- Svgy selects the 256 colors from the render.
- Each pixel is fully transparent or fully opaque. Pixels with less than 50% opacity become
  transparent.
- Because of this, the edges of the artwork are not smooth.

**Technical details:**

- The width and height fields are one byte each. The value 0 means 256.
- Each BMP is a bottom-up 8 bpp DIB: a table of 256 colors, the pixels, then a 1-bit AND mask.
- A median-cut palette selects the 256 colors.
- Each color table is 1 KiB. Svgy writes the full table, because old readers expect it.

## Linux icons

References:

- [Icon Theme Specification](https://specifications.freedesktop.org/icon-theme/latest/)
- AppImage: [The filesystem image](https://github.com/AppImage/AppImageSpec/blob/master/draft.md#the-filesystem-image)

Installation paths:

- SVG: `/usr/share/icons/hicolor/scalable/apps/<app-id>.svg`, such as `com.ystorian.svgy.svg`.
- PNG: `/usr/share/icons/hicolor/256x256/apps/<app-id>.png`, such as `com.ystorian.svgy.png`.
