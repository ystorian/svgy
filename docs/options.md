# Options

```shell
svgy <input.svg> [targets] [options]
```

## Words on this page

- **Target**: A file that Svgy makes, such as a PNG or an icon.
- **Artwork**: The shapes in the SVG.
- **Canvas**: The area around the artwork. The SVG `viewBox` sets it.
- **Render**: The image that you see when you open the SVG.

## Targets

- `--svg[=<path>]`: Make a small SVG.
- `--png[=<path>]`: Make a PNG.
- `--ico[=<path>]`: Make a Windows icon (`.ico`).
- `--icns[=<path>]`: Make a macOS icon (`.icns`).
- `--round[=<path>]`: Make an SVG with the artwork inside a circle.

**Notes:**

- With no target, Svgy makes an SVG.
- When you give a target, Svgy does not make the SVG. Add `--svg` to get it too.
- `--round` has no effect on the other targets.
  - `svgy logo.svg --round --icns` makes a round SVG and a normal icon.
  - For a round icon, run Svgy two times. Use the round SVG as the input the second time.

## Size

These options are not mandatory. They do not change the shape of the artwork.

- `--size=<pixels>`: Make the image fit in a square. The default is `1024`.
- `--width=<pixels>`: Set the width.
- `--height=<pixels>`: Set the height.
- `--no-resize`: Keep the original size.
- `--no-square`: Make the canvas the same size as the artwork. Do not add empty space.

**Notes:**

- Svgy makes the longest side of the artwork the same as the size.
- Svgy adds empty space to make a square canvas. It puts the artwork in the center.
  - Example: 620 x 720 with `--size=1024` gives a canvas of 1024 x 1024.
  - With `--no-square`, it gives 882 x 1024.
- You can use only one of `--size`, `--width` and `--height`.
- `--width` and `--height` together: the artwork fits in that rectangle.
- `--width` or `--height` alone: Svgy does not add empty space.
- These options have no effect on `.ico` and `.icns`. Icons always have the same sizes. Refer to
  [Formats](formats.md).
- When the input is not square, Svgy puts the artwork in the center of each icon.

## Round

- `--padding[=<value>]`: Add empty space between the artwork and the circle.
  - The value is from `0.0` to less than `1.0`. `0.1` is 10% of the circle radius.
  - The default is `0.0`.
  - When the input has a background that fills the canvas, the default is `0.1`.
  - `--padding` with no value is `0.1`.
  - Svgy tells you which default it used.

`--padding` works only with `--round`.

## File names

When you do not give a path, Svgy puts the file next to the input. The name is
`<name>.<suffix>.<extension>`.

- `svgy example.svg --png` makes `example.svgy.png`.
- `svgy example.svg --icns` makes `example.svgy.icns`.

The default suffix is `svgy`. Because of the suffix, Svgy does not write over the input.

- `--suffix=<suffix>`: Use a different suffix.
  - `svgy example.svg --png --suffix=v2` makes `example.v2.png`.

The suffix of `--round` is `round`. Because of this, you can use `--svg` and `--round` together.

- `svgy example.svg --svg --round` makes `example.svgy.svg` and `example.round.svg`.

## Precision

Svgy removes decimals from the numbers in the SVG. This makes the file smaller. Too few decimals
change the render.

- `--precision[=<value>]`: The maximum change to the render.
  - The default is `0.02`, which is 2%.
  - `--precision` with no value is `0.02`.
- `--no-precision`: Keep 3 decimals for all numbers.

**Notes:**

- Svgy tells you how many decimals it kept, such as `precision 0 to 1 (6 refined)`.
- When no value is good, Svgy shows a warning. Then it keeps 3 decimals.
- This option changes only the SVG. It has no effect on PNG and icons.
- It takes less than one second.
- Refer to [Optimization](optimization.md#precision-search) for the details.

## Speed and file size

- `--no-optimize`: Do not make the files smaller. Svgy is much faster. Use it while you work on an
  icon.
- `--zopfli`: Make the PNG files 1% to 5% smaller. Svgy is 100 times slower.
- `--no-legacy-ico`: Remove the old 256-color images from the Windows icon.
  - The icon is about 3.6 KiB smaller.
  - Very old Windows versions cannot show the icon.

## Images

- `--strip-images`: Remove all bitmap images (`<image>`) from the input, and continue.
  - Without this option, Svgy stops when it finds an image.
  - Svgy shows a warning when it removes an image.
