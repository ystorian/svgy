# .just/svgy.just

# Variables.
tests_dir := 'tests'
fictures := tests_dir + '/fictures'
results := tests_dir + '/results'
test_files := ['ally', 'dev', 'svgy', 'geneva', 'jura', 'loki', 'road-closure']
oxvg_config := tests_dir + '/oxvg.json'

# Suffix + extensions.
rsvg := '.rsvg.svg'
rsvg_oxvg := '.rsvg_oxvg.svg'
svgy := '.svgy.svg'
no_opt := '.svgy_no_optimize.svg'
no_pre := '.svgy_no_precision.svg'
no_opt_oxvg := '.svgy_no_optimize_oxvg.svg'
no_pre_oxvg := '.svgy_no_precision_oxvg.svg'
round := '.svgy_round.svg'

# Functions.
ficture(svg) := fictures + '/' + svg + '.svg'
result(svg) := results + '/' + svg + '/' + svg + '.svg'
rsvg(svg) := results + '/' + svg + '/' + svg + rsvg
no_opt(svg) := results + '/' + svg + '/' + svg + no_opt
no_pre(svg) := results + '/' + svg + '/' + svg + no_pre
rsvg_oxvg(svg) := results + '/' + svg + '/' + svg + rsvg_oxvg
no_pre_oxvg(svg) := results + '/' + svg + '/' + svg + no_pre_oxvg
no_opt_oxvg(svg) := results + '/' + svg + '/' + svg + no_opt_oxvg
svgy(svg) := results + '/' + svg + '/' + svg + svgy
round(svg) := results + '/' + svg + '/' + svg + round

# Display the size of `svg`.
svg_size(color, text, name, ext) := '"' + style(color, '  ' + replace_regex(text + '                        ', '^(.{24,}?) *$', '$1') + '  ' + BOLD + file_size(results + '/' + name + '/' + name + ext)) + style(['dim', 'italic'], ' \t' + results + '/' + name + '/' + name + ext) + '"'
# Heading for sizes.
list_heading(color, text) := '"' + style([color, 'dim'], text) + '"'

# Resize and optimize the test files
[group('Svgy')]
test_resize: install
	just create_dir {{ results }}

	echo {{ dim(YELLOW + 'Create per-file subdirectories:') }}; \
	for svg in {{ test_files }}; do \
		just create_dir {{ results }}/$svg; \
	done

	echo {{ dim(['Copy', GREEN + fictures, '->', YELLOW + results]) }}
	for svg in {{ test_files }}; do \
		cp {{ ficture("$svg") }} {{ result("$svg") }}; \
	done

	echo {{ dim(['Resize with', BOLD + 'rsvg', '->', YELLOW + rsvg]) }}
	for svg in {{ test_files }}; do \
		just resize_svg {{ ficture("$svg") }} {{ rsvg("$svg") }}; \
	done

	echo {{ dim(['Resize with', BOLD + 'svgy --no-optimize', '->', YELLOW + no_opt]) }}
	for svg in {{ test_files }}; do \
		svgy --no-optimize --svg={{ no_opt("$svg") }} {{ ficture("$svg") }} >/dev/null; \
	done

	echo {{ dim(['Resize with', BOLD + 'svgy --no-precision', '->', YELLOW + no_pre]) }}
	for svg in {{ test_files }}; do \
		svgy --no-precision --svg={{ no_pre("$svg") }} {{ ficture("$svg") }} \
		>/dev/null; \
	done

	echo {{ dim(['Optimize', BOLD + 'rsvg', 'with', BOLD + 'oxvg', '->', YELLOW + rsvg_oxvg]) }}
	for svg in {{ test_files }}; do \
		just optimize_svg {{ rsvg("$svg") }} {{ rsvg_oxvg("$svg") }}; \
	done

	echo {{ dim(['Optimize', BOLD + 'svgy --no-precision', 'with', BOLD + 'oxvg', '->', YELLOW + no_pre_oxvg]) }}
	for svg in {{ test_files }}; do \
		just optimize_svg {{ no_pre("$svg") }} {{ no_pre_oxvg("$svg") }}; \
	done

	echo {{ dim(['Optimize', BOLD + 'svgy --no-optimize', 'with', BOLD + 'oxvg', '->', YELLOW + no_opt_oxvg]) }}
	for svg in {{ test_files }}; do \
		just optimize_svg {{ no_opt("$svg") }} {{ no_opt_oxvg("$svg") }}; \
	done

	echo {{ dim(['Resize and optimize with', BOLD + 'svgy', '->', YELLOW + svgy]) }}
	for svg in {{ test_files }}; do \
		svgy --svg={{ svgy("$svg") }} {{ ficture("$svg") }} >/dev/null; \
	done

	echo {{ dim(['Round with', BOLD + 'svgy --round', '->', YELLOW + round]) }}
	for svg in {{ test_files }}; do \
		svgy --round={{ round("$svg") }} {{ ficture("$svg") }} >/dev/null; \
	done

	just list_resized

# List all resized SVG sizes
[group('Svgy')]
[private]
[script]
list_resized:
	for svg in {{ test_files }}; do
		if [ ! -d "{{ results }}/$svg" ]; then
			echo {{ error(['Directory not found:', BOLD + results + "/$svg"]) }};
			exit 1;
		fi
		just list_sizes "$svg"
	done

# List the sizes of an SVG in its `results` directory
[group('Svgy')]
[private]
list_sizes svg:
	if [ ! -d "{{ results }}/{{ svg }}" ]; then \
		echo {{ error(['Directory not found:', BOLD + results + '/' + svg]) }}; \
		exit 1; \
	fi

	echo "\n{{ style(['bold', 'reverse', 'blue'], '  ' + svg + '  ') }}"
	echo {{ svg_size('blue', 'Original', svg, '.svg') }}

	echo {{ list_heading('yellow', 'Resize only') }}
	echo {{ svg_size('yellow', 'rsvg', svg, rsvg) }}
	echo {{ svg_size('yellow', 'svgy --no-optimize', svg, no_opt) }}
	echo {{ svg_size('yellow', 'svgy --no-precision', svg, no_pre) }}

	echo {{ list_heading('magenta', 'Optimize with ' + BOLD + 'oxvg') }}
	echo {{ svg_size('magenta', 'rsvg', svg, rsvg_oxvg) }}
	echo {{ svg_size('magenta', 'svgy --no-optimize', svg, no_opt_oxvg) }}
	echo {{ svg_size('magenta', 'svgy --no-precision', svg, no_pre_oxvg) }}

	echo {{ list_heading('green', 'Optimize with ' + BOLD + 'svgy') }}
	echo {{ svg_size('green', 'svgy', svg, svgy) }}

	echo {{ list_heading('cyan', 'Round with ' + BOLD + 'svgy') }}
	echo {{ svg_size('cyan', 'svgy --round', svg, round) }}

# Convert with `rsvg-convert` an SVG to PNG
[group('Svgy')]
[private]
svg_to_png svg png: check_command_rsvg-convert
	rsvg-convert \
		--format png \
		--keep-aspect-ratio \
		--width=1024 \
		--height=1024 \
		--output {{ png }} \
		{{ svg }}

# Resize with `rsvg-convert` an SVG to 1024x1024
[group('Svgy')]
[private]
resize_svg svg resized: check_command_rsvg-convert
	rsvg-convert \
		--format svg \
		--keep-aspect-ratio \
		--width=1024 \
		--height=1024 \
		--output {{ resized }} \
		{{ svg }}

# Compress with `oxipng` a PNG
[group('Svgy')]
[private]
compress_png png compressed: check_command_oxipng
	oxipng \
		--strip safe \
		--alpha \
		--opt 4 \
		--out {{ compressed }} \
		{{ png }}

# Optimize with `oxvg` an SVG
[group('Svgy')]
[private]
optimize_svg svg optimized: check_command_oxvg
	oxvg \
		optimise \
		--quiet \
		--config {{ oxvg_config }} \
		--pretty tabs \
		--output {{ optimized }} \
		{{ svg }}

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
