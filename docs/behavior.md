# Behavior

## Steps

Svgy does these steps in this order:

1. Read the input.
2. Look for bitmap images.
3. Remove text.
4. Change the size.
5. Fit in a circle (`--round` only).
6. Make the files smaller.
7. Write the files.

Svgy makes the PNG and the icons after step 4. Steps 5 and 6 have no effect on their pixels.

## Text

Svgy removes all text from the input. It shows a warning when it does this.

Why: Svgy removes the text from all files. Then all files look the same.

Before you use Svgy, change the text into shapes. In Inkscape:

1. Select the text.
2. Click **Path > Object to Path** (`Shift+Ctrl+C`).

## Images

When the input has a bitmap image (`<image>`), Svgy stops.

Why: Svgy cannot draw a bitmap in the PNG and the icons. The files would not look the same.

To continue, do one of these steps:

- Change the image into shapes.
- Use `--strip-images`. Svgy removes the image and continues.

## Files

- Svgy reads one input at a time. To convert many files, use a shell loop.
- Svgy makes the folders of the output path if they do not exist.
- Svgy writes over files that exist. It does not ask first.

## Exit codes

- `0`: Success.
- `1`: Error. Svgy shows a message.

## Platforms

Svgy works the same on Linux, macOS and Windows. You can make a macOS icon on Windows.
