# .just/ci.just


# Run the CI tests like in `.github/workflows/ci-rust.yaml`
[group('CI')]
[env("CARGO_TERM_QUIET", "true")]
[env("CARGO_INCREMENTAL", "0")]
[env("CARGO_PROFILE_TEST_DEBUG", "0")]
[env("CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS", "")]
[env("CARGO_TARGET_AARCH64_PC_WINDOWS_MSVC_RUSTFLAGS", "")]
ci:
	echo {{CYAN}}Running {{GREEN}}CI checks{{CYAN}} in silent mode{{NORMAL}}

	echo {{YELLOW}}cargo fmt{{NORMAL}}
	cargo fmt --all
	echo {{done('cargo fmt')}}

	echo {{YELLOW}}cargo-deny{{NORMAL}}
	cargo-deny --all-features --workspace --log-level error check
	echo {{done('cargo-deny')}}

	echo {{YELLOW}}cargo test{{NORMAL}}
	cargo test --frozen --profile ci --all-features --workspace
	echo {{done('cargo test')}}

	echo {{YELLOW}}cargo clippy{{NORMAL}}
	cargo clippy --frozen --profile ci --all-features --workspace --tests -- -D warnings
	echo {{done('cargo clippy')}}

	echo {{done('')}}


# Sync the packager configs with `Cargo.toml` and stage them
[group('CI')]
[private]
precommit:
	# Run by `.githooks/pre-commit` and set by `just install_git_hooks`

	# Sync `Cargo.toml` version in `Cargo.lock`.
	cargo update --offline

	# Format Rust code.
	cargo fmt --all

	echo {{done('')}}
