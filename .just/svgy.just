# .just/svgy.just

# Variables.
tests_dir := 'tests'
source_dir := tests_dir + '/fictures'
result_dir := tests_dir + '/result'
test_files := ['ally', 'svgy', 'geneva', 'jura']
resize_dir := result_dir + '/resize'
oxvg_config := tests_dir + '/oxvg.json'

# Functions.

# Display the size of `svg`
svg_size(color, text, svg) := style(color, '  ' + text + '  ') + style([color, 'bold'], file_size(svg)) + style([color, 'italic'], '  ' + file_name(svg))

# Resize test
[group('Svgy')]
test_resize:
	just install

	just create_dir {{ result_dir }}
	just create_dir {{ resize_dir }}

	echo "{{ CYAN }}Copy: {{ GREEN + source_dir + CYAN }} -> {{ YELLOW + resize_dir + NORMAL }}"
	for svg in {{ test_files }}; do \
		just copy_file {{ source_dir }}/$svg.svg {{ resize_dir }} \
	;done

	echo "{{ CYAN }}Resize: rsvg -> {{ YELLOW }}.rsvg.svg{{ NORMAL }}"
	for svg in {{ test_files }}; do \
		just resize_svg {{ resize_dir }}/$svg.svg {{ resize_dir }}/$svg.rsvg.svg \
	;done

	echo "{{ CYAN }}Optimize: rsvg + oxvg -> {{ YELLOW }}.rsvg.oxvg.svg{{ NORMAL }}"
	for svg in {{ test_files }}; do \
		just optimize_svg {{ resize_dir }}/$svg.rsvg.svg {{ resize_dir }}/$svg.rsvg.oxvg.svg \
	;done

	echo "{{ GREEN }}Resize and optimize: svgy -> {{ YELLOW }}.svgy.svg{{ NORMAL }}"
	for svg in {{ test_files }}; do \
		svgy --svg={{ resize_dir }}/$svg.svgy.svg {{ resize_dir }}/$svg.svg \
		;done

	echo "{{ CYAN }}Resize: svgy --no-optimize -> {{ YELLOW }}.svgy.no-optimize.svg{{ NORMAL }}"
	for svg in {{ test_files }}; do \
		svgy --no-optimize --svg={{ resize_dir }}/$svg.svgy.no-optimize.svg {{ resize_dir }}/$svg.svg \
		;done

	echo "{{ CYAN }}Resize: svgy --no-precision -> {{ YELLOW }}.svgy.no-precision.svg{{ NORMAL }}"
	for svg in {{ test_files }}; do \
		svgy --no-precision --svg={{ resize_dir }}/$svg.svgy.no-precision.svg {{ resize_dir }}/$svg.svg \
		;done

	echo "{{ CYAN }}Optimize: svgy --no-precision + oxvg -> {{ YELLOW }}.svgy.no-precision.oxvg.svg{{ NORMAL }}"
	for svg in {{ test_files }}; do \
		just optimize_svg {{ resize_dir }}/$svg.svgy.no-precision.svg {{ resize_dir }}/$svg.svgy.no-precision.oxvg.svg \
		;done

	echo "{{ CYAN }}Optimize: svgy --no-optimize + oxvg ->{{ YELLOW }}.svgy.no-optimize.oxvg.svg{{ NORMAL }}"
	for svg in {{ test_files }}; do \
		just optimize_svg {{ resize_dir }}/$svg.svgy.no-optimize.svg {{ resize_dir }}/$svg.svgy.no-optimize.oxvg.svg \
	;done

	just list_resized

# List all resized file sizes
[group('Svgy')]
[private]
list_resized:
	for svg in {{ test_files }}; do \
		just list_sizes {{ resize_dir }}/$svg.svg \
	;done

# List the sizes of an `svg`
[group('Svgy')]
list_sizes svg:
	echo "\n{{ UNDERLINE + BOLD + BLUE + svg + NORMAL }}"
	echo "{{ svg_size('blue', 'Source SVG              ', svg) }}"

	echo "{{ UNDERLINE + YELLOW + 'Resize:' + NORMAL }}"
	echo "{{ svg_size('yellow', 'rsvg                    ', without_extension(svg) + '.rsvg.svg') }}"
	echo "{{ svg_size('cyan', 'svgy --no-optimize      ', without_extension(svg) + '.svgy.no-optimize.svg') }}"
	echo "{{ svg_size('cyan', 'svgy --no-precision     ', without_extension(svg) + '.svgy.no-precision.svg') }}"

	echo "{{ UNDERLINE + YELLOW + 'Resize + Optimize (oxvg):' + NORMAL }}"
	echo "{{ svg_size('yellow', 'rsvg with oxvg          ', without_extension(svg) + '.rsvg.oxvg.svg') }}"
	echo "{{ svg_size('cyan', '--no-optimize with oxvg ', without_extension(svg) + '.svgy.no-optimize.oxvg.svg') }}"
	echo "{{ svg_size('cyan', '--no-precision with oxvg', without_extension(svg) + '.svgy.no-precision.oxvg.svg') }}"

	echo "{{ UNDERLINE + GREEN + 'Optimize (svgy):' + NORMAL }}"
	echo "{{ svg_size('green', 'svgy                    ', without_extension(svg) + '.svgy.svg') }}"

# Convert `svg` to `png`
[group('Svgy')]
[private]
svg_to_png svg png:
	just check_command_rsvg-convert
	rsvg-convert \
		--format png \
		--keep-aspect-ratio \
		--width=1024 \
		--height=1024 \
		--output {{ png }} \
		{{ svg }}
	echo "{{ dim(png + ' ' + file_size(png)) }}"

# Resize `svg` to 1024x1024 `new_svg`
[group('Svgy')]
[private]
resize_svg svg new_svg:
	just check_command_rsvg-convert
	rsvg-convert \
		--format svg \
		--keep-aspect-ratio \
		--width=1024 \
		--height=1024 \
		--output {{ new_svg }} \
		{{ svg }}
	echo "{{ dim(new_svg + ' ' + file_size(new_svg)) }}"

# Compress `png` to `new_png`
[group('Svgy')]
[private]
compress_png png new_png:
	just check_command_oxipng
	oxipng \
		--strip safe \
		--alpha \
		--opt 4 \
		--out {{ new_png }} \
		{{ png }}
	echo "{{ dim(new_png + ' ' + file_size(new_png)) }}"

# Optimize `svg` to `new_svg` with `oxvg`
[group('Svgy')]
[private]
optimize_svg svg new_svg:
	just check_command_oxvg
	oxvg \
		optimise \
		--quiet \
		--config {{ oxvg_config }} \
		--pretty tabs \
		--output {{ new_svg }} \
		{{ svg }}
	echo "{{ dim(new_svg + ' ' + file_size(new_svg)) }}"

# Check if `oxvg` is available
[group('Svgy')]
[private]
check_command_oxvg:
	just check_command oxvg 'cargo binstall oxvg' 'cargo binstall oxipng'

# Check if `oxipng` is available
[group('Svgy')]
[private]
check_command_oxipng:
	just check_command oxipng 'cargo binstall oxipng' 'cargo binstall oxipng'

# Check if `rsvg-convert` is available
[group('Svgy')]
[private]
check_command_rsvg-convert:
	just check_command rsvg-convert 'brew install rsvg-convert' ''
