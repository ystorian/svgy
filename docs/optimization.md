# Optimization

This page tells how Svgy makes files smaller. You do not need it to use Svgy.

## Summary

- **SVG**: Svgy uses `oxvg` to make the SVG smaller. Then it removes decimals from the numbers. It
  stops before the render changes too much.
- **PNG and icons**: Svgy draws the images with `resvg`. Then it makes them smaller with `oxipng`.

## Steps for each target

**SVG:**

1. Change the size. Svgy changes the numbers of the shapes. It does not add a `transform`.
2. Keep 6 decimals.
3. Make the SVG smaller with `oxvg`.
4. Find the smallest number of decimals. Refer to [Precision search](#precision-search).

**Round SVG:**

1. Find the smallest circle around the artwork. Svgy ignores backgrounds.
2. Move and scale the artwork so that this circle fits the canvas.
3. Make the SVG smaller with `oxvg`.
4. Find the smallest number of decimals.

**PNG:**

1. Draw the image with `resvg`.
2. Make it smaller with `oxipng`.

**Windows icon:**

1. Draw each size with `resvg`.
2. Make PNG images, and make them smaller with `oxipng`.
3. For 16 x 16 and 32 x 32, also make BMP images with 256 colors.

**macOS icon:**

1. Draw each size with `resvg`.
2. For 16 x 16 and 32 x 32, make ARGB-RLE images.
3. For the other sizes, make PNG images, and make them smaller with `oxipng`.

Refer to [Formats](formats.md) for the images in each icon.

## Size change

Svgy does not add a `transform` to change the size. It multiplies each number of each shape.

- The render does not change.
- The structure of the SVG does not change.
- Svgy changes the `transform` values that exist. It does not remove them.
- A `<symbol>` keeps its numbers. The `<use>` that shows it gets the change.
- The `viewBox` always starts at `0 0`.

After this step, `oxvg` can change the structure. For example, it can move a `transform` to a group.

## SVG

Svgy runs `oxvg` with its default jobs. A job is one type of change.

- The output keeps one element on each line, with tabs.
- Svgy removes `width` and `height`. With `--no-resize`, it keeps them.

### Extra jobs

Svgy adds three `oxvg` jobs:

- `removeXlink`: Change `xlink:href` to `href`.
- `removeAttrs`: Remove `xml:space`. It has no use after Svgy removes the text. When it stays, the
  output has no tabs.
- `convertStyleToAttrs`: Move values from `style` to attributes. Then `oxvg` can remove the values
  that are the same as the default.

### Svgy cleanups

Svgy also does five changes of its own, in this order:

1. **Root `id`**: Remove the `id` of the top element, if nothing uses it.
   - Why: `oxvg` does not clean an element that has an `id`. This `id` would stop many cleanups.
2. **Gradient pairs**: Join two gradients into one. Refer to [Gradient pairs](#gradient-pairs).
3. **Gradient transforms**: Remove a `gradientTransform`. Refer to
   [Gradient transforms](#gradient-transforms).
4. **Opacity**: Keep 3 decimals for an opacity in a `style` attribute.
   - Why: `oxvg` does not round these values. Editors write values such as `0.98039216`.
   - 3 decimals are enough for an opacity.
5. **Namespaces**: Remove the namespaces that nothing uses, such as `xmlns:svg`.
   - Why: Editors leave them, and `oxvg` writes them on each element.
   - Svgy always keeps the default `xmlns`.

When the SVG has a `<style>` or a `<script>`, Svgy keeps the `style` attributes and the root `id`.
These elements can use them, and Svgy cannot check this.

### Gradient pairs

Editors often write one gradient as two elements:

- The first gradient has the colors (the stops).
- The second gradient has the position. It points to the first with `href`.

Svgy joins them into one element. This saves about 37 bytes for each pair.

Svgy joins them only when:

- Only one gradient points to the first gradient.
- No shape uses the first gradient directly.

The two gradients can be of different types (linear and radial). Svgy copies only the attributes
that the result can use.

### Gradient transforms

Svgy puts the `gradientTransform` into the position of the gradient. Then it removes the
`gradientTransform`.

- **Linear gradient**: Svgy can do this for all transforms.
- **Radial gradient**: Svgy can do this only for a move, a rotation, or the same scale on both
  axes. Other transforms change the circle into an oval. A radial gradient cannot show an oval.

Svgy keeps the `gradientTransform` when a position is a percentage or is missing.

### Many runs

Svgy runs all jobs up to three times.

- Why: Rounding can put two lines in a straight line. The next run can then join them.
- When a run does not make the file smaller, Svgy ignores that run.
- Two runs are enough for all test files.

## Precision search

Svgy removes decimals to make the SVG smaller. Too few decimals change the render. Svgy finds the
smallest number of decimals that is still correct.

1. Make the SVG with 0, 1, 2, 3, 4 and 5 decimals.
2. Draw the input and each result at 1024 x 1024.
3. Compare each result to the input.
4. Keep the first result with a change less than `--precision`.

**Notes:**

- Fewer decimals give a larger change. Because of this, the first good result is the smallest.
- Artwork with whole numbers can have 0 decimals.
- Artwork with many details needs more decimals.
- `--round` often needs one more decimal than `--svg`, because it moves the shapes.
- A small change stays at 5 decimals. The `mergePaths` job has its own limit, and Svgy cannot
  change it.

### Different decimals for each element

With one number of decimals for all elements, each element gets what the worst element needs. Svgy
can do better:

1. Give each element the smallest number of decimals.
2. Add one decimal to one element.
3. Draw the result. Keep the change if the render is still correct.
4. Do steps 2 and 3 again for the next element.

Svgy shows the result, such as `precision 0 to 1 (6 refined)`. Here, the elements have 0 or 1
decimals, and 6 elements got more decimals.

**Which element first:**

- Svgy starts with the elements that have the longest edges for the fewest bytes.
- Gradients have no edges. They are last.
- This order can cost some bytes. It cannot make the render wrong, because Svgy checks each step.

**When Svgy does not do this:**

- The results at different decimals have a different structure. `mergePaths` can cause this.
- No mix is correct.
- The mix is not smaller.

In these cases, Svgy keeps the result of the first search.

**Result:** The change to the render is often near the `--precision` value. For a more correct
render, use a smaller value.

**Time:** Up to 6 draws for the search, and up to 32 for the mix. This is less than one second.

## PNG

Svgy runs `oxipng` at level **4** on each PNG.

- `--zopfli`: Makes each PNG 1% to 5% smaller (2% on average). This takes minutes, not seconds.
- `--no-optimize`: Do not make the PNG smaller.
