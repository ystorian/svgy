# @template/.just/rust.just

# Get the app version from `Cargo.toml`, with or without workspace.
# Requires `tomli`, installed by `just install_cargo_tools`.
version := trim(replace(`tomli query --filepath Cargo.toml workspace.package.version 2>/dev/null ||
	tomli query --filepath Cargo.toml package.version`, '"', ''))

# Release tag `v#.#.#`.
tag := 'v' + version

# Display "in quiet mode" for non-release profiles.
display_profile(profile) := if profile != 'release' { NORMAL + YELLOW + ' ' + profile + style('dim') + ' in quiet mode' + set_title('🦀 ' + recipe_name()) + NORMAL } else { NORMAL + GREEN + ' release ' + set_title('🦀 ' + recipe_name()) + NORMAL }

# Benchmark
[group('Rust')]
bench profile='dev':
	echo "{{ CYAN }}Run {{ BOLD }}cargo bench{{ display_profile(profile) }}"
	cargo bench --all-features --workspace \
		{{ if profile != 'release' { '--profile ' + profile + ' --quiet' } }}
	echo "{{ set_title('') + done(profile) }}"

# Check for errors
[group('Rust')]
check profile='dev':
	echo "{{ CYAN }}Run {{ BOLD }}cargo check{{ display_profile(profile) }}"
	cargo check --workspace --all-features --future-incompat-report \
		{{ if profile != 'release' { '--profile ' + profile + ' --quiet' } else { '--release' } }}
	echo "{{ set_title('') + done(profile) }}"

# Check dependencies and licenses
[group('Rust')]
deny:
	echo {{ CYAN }}Run {{ BOLD }}cargo-deny{{ NORMAL }}
	cargo-deny --all-features --workspace --log-level error check
	echo {{ done('') }}

# Format Rust code
[group('Rust')]
format:
	echo {{ CYAN }}Run {{ BOLD }}cargo fmt{{ NORMAL }}
	cargo fmt --all
	echo {{ done('') }}
alias fmt := format

# Clippy
[group('Rust')]
clippy:
	echo "{{ CYAN }}Run {{ BOLD }}cargo clippy{{ NORMAL }}"
	cargo clippy --all-targets --all-features --all -- --deny clippy::all --warn clippy::pedantic
	echo {{ done('') }}
alias lint := clippy

# Clippy with only `lint`
[env("CARGO_TERM_QUIET", "true")]
[group('Rust')]
clippy_lint lint:
	echo {{ CYAN }}Run {{ BOLD }}cargo clippy{{ NORMAL + CYAN }} for the \
		{{ YELLOW + lint + CYAN }} lint{{ NORMAL }}
	cargo clippy --all-targets --all-features --all -- --warn clippy::{{ lint }}
	echo {{ done(lint) }}

# Fix easy Clippy lints (requires no uncommitted changes)
[env("CARGO_TERM_QUIET", "true")]
[group('Rust')]
clippy_fix:
	echo {{ CYAN }}Fix {{ ITALIC }}easy{{ NORMAL + CYAN }} Clippy lints{{ NORMAL }}
	echo ⚠️ {{ YELLOW }}Commit all your changes first{{ NORMAL }}
	cargo clippy --fix
	echo {{ done('') }}

# Run unit and integration tests
[group('Rust')]
[no-exit-message]
test profile='dev':
	echo "{{ CYAN }}Run {{ BOLD }}cargo test{{ display_profile(profile) }}"
	if [ "{{ profile }}" != "release" ]; then \
		if ! cargo test --all-features --workspace --future-incompat-report --quiet --profile {{ profile }}\
		>/dev/null 2>&1; then \
			cargo test --all-features --workspace --future-incompat-report --profile {{ profile }}; \
		fi; \
	else \
		cargo test --all-features --workspace --future-incompat-report --release; \
	fi
	echo "{{ set_title('') + done(profile) }}"

# Clean the workspace
[group('Rust')]
clean_workspace:
	echo {{ CYAN }}Clean the workspace{{ NORMAL }}

	cargo-clean-all \
		--keep-empty-target \
		--keep-executable-same-dir \
		--yes \
		.
	just deny

	echo {{ done('') }}

# Build and install on this machine
[group('Rust')]
install:
	echo {{ CYAN }}Run {{ BOLD }}cargo install{{ NORMAL }}
	cargo install --path .
	echo {{ done('') }}

# Download the crates in `Cargo.toml`
[group('Rust')]
fetch:
	echo {{ CYAN }}Run {{ BOLD }}cargo fetch{{ NORMAL }}
	cargo fetch --locked --verbose
	echo {{ done('') }}
