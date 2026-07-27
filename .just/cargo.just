# .just/cargo.just


# Variables.
# cargo binstall.
need_binstall := if which("cargo-binstall") == [] {'true'} else {'false'}
binstall_options := '--locked --no-confirm --only-signed --disable-telemetry --no-discover-github-token'
native_target := (
	if os() == 'windows' 	{arch() + '-pc-windows-msvc'}
	else if os() == 'macos' {arch() + '-apple-darwin'}
	else if os() == 'linux' {arch() + '-unknown-linux-gnu'}
	else 					{arch()}
)
cargo_tools := [
	'cargo-clean-all',
	'cargo-deny',
	'cargo-update',
	'dockerfile-roast',
	'just',
	'zizmor',
]
project_tools := [
	'cargo-packager',
]
windows_tools := if os() == 'windows' {[]}

# Install cargo tools
[group('Cargo')]
install_cargo:
	echo {{CYAN}}Install cargo tools{{NORMAL}}

	if {{need_binstall}} ; then \
		echo {{BLUE}}Install {{GREEN}}cargo-binstall{{NORMAL}} && \
		cargo install --locked --targets {{native_target}} cargo-binstall; fi

	echo {{CYAN}}Install the required cargo tools{{NORMAL}}
	cargo-binstall \
		{{binstall_options}} \
		--targets {{native_target}} \
		{{join_list(cargo_tools)}} \
		{{join_list(project_tools)}} \
		{{join_list(windows_tools)}}

	echo {{CYAN}}Update all cargo tools{{NORMAL}}
	cargo install-update --all
	echo {{done('')}}


# Update all
[group('Cargo')]
update_all:
	echo {{BLUE}}Update all{{NORMAL}}
	just update_rust
	just update_cargo
	just bump_deps
	echo {{done('')}}


# Update Rust
[group('Cargo')]
update_rust:
	echo {{CYAN}}Update Rust{{NORMAL}}
	rustup self update
	rustup update stable
	rustup component add rust-analyzer
	echo {{done('')}}


# Update cargo tools
[group('Cargo')]
update_cargo:
	echo {{CYAN}}Update cargo tools{{NORMAL}}
	cargo install-update --all --git --locked --cooldown 7d
	echo {{done('')}}


# Update dependencies
[group('Cargo')]
bump_deps:
	echo {{CYAN}}Update dependencies in {{GREEN}}Cargo.toml{{NORMAL}}
	cargo update --verbose
	just deny
	echo {{done('')}}


# List cargo tools installed with the correct target arch
[group('Cargo')]
[unix]
cargo_native:
	echo {{CYAN}}Installed native {{GREEN + arch_short + CYAN}} cargo tools{{NORMAL}}
	find $CARGO_HOME/bin -type f | xargs file | grep -v "for architecture" | grep "{{arch_short}}"
	echo {{done('')}}


# List cargo tools that should be reinstalled with the native target arch
[group('Cargo')]
[windows]
cargo_non_native:
	echo {{CYAN}}List {{YELLOW}}non-native {{arch_short + CYAN}} cargo tools{{NORMAL}}
	just delete_file "{{env('TEMP')}}/sigcheck.txt"
	echo {{CYAN}}Running {{BOLD}}sigcheck{{NORMAL}}
	sigcheck -c -r -e -nobanner -w "{{env('TEMP')}}/sigcheck.txt" "{{env('CARGO_HOME')}}/bin" || true
	dos2unix --quiet "{{env('TEMP')}}/sigcheck.txt"
	xsv search --invert-match ARM --select 10 "{{env('TEMP')}}/sigcheck.txt" | xsv select 1,3,10 | xsv table
	just delete_file "{{env('TEMP')}}/sigcheck.txt"
	echo {{done('')}}


# List cargo tools that must be reinstalled with the native architecture
[group('Cargo')]
[no-exit-message]
[unix]
cargo_non_native:
	echo {{CYAN}}List non-native {{YELLOW + arch_short + CYAN}} cargo tools{{NORMAL}}
	find $CARGO_HOME/bin -type f \
		| xargs file \
		| grep -v "for architecture" \
		| grep -v "{{arch_short}}" && \
		echo "⚠️ {{YELLOW}} Reinstall them with {{BOLD+GREEN}}just cargo_reinstall <tool>{{NORMAL}}" || \
		echo {{GREEN+ITALIC}}All cargo tools are native{{NORMAL}}
	echo {{done('')}}


# Force-reinstall a cargo tool with the native architecture
[group('Cargo')]
cargo_reinstall tool:
	echo {{CYAN}}Force-reinstall {{YELLOW + tool + CYAN}} for {{GREEN + arch_short + NORMAL}}
	cargo binstall {{binstall_options}} --targets {{native_target}} --force {{tool}}
	echo {{done(tool)}}


# Install Visual Studio build tools on Windows
[group('Cargo')]
[windows]
install_vs_build_tools:
	echo {{CYAN}}Install {{YELLOW}}Microsoft Visual Studio build tools{{NORMAL}}
	winget install \
		--id Microsoft.VisualStudio.Community \
		--accept-source-agreements \
		--accept-package-agreements \
		--force \
		--override "\
			--add Microsoft.VisualStudio.Component.VC.Llvm.Clang \
			--add Microsoft.VisualStudio.Component.VC.Llvm.ClangToolset \
			--add Microsoft.VisualStudio.Component.VC.Redist.14.Latest \
			--add Microsoft.VisualStudio.Component.VC.Tools.ARM64 \
			--add Microsoft.VisualStudio.Component.VC.Tools.x86.x64 \
			--add Microsoft.VisualStudio.Component.Windows10SDK \
			--add Microsoft.VisualStudio.Component.Windows11SDK.28000 \
			--addProductLang En-us"
	echo {{done('')}}


# Verify the crate is publishable with a dry run
[group('Cargo')]
[no-exit-message]
publish_check:
	echo {{CYAN}}Check if this crate is publishable{{NORMAL}}

	just check_tag {{tag}}

	echo "{{dim('Cargo.toml -> [package] -> publish')}}"
	if [ "$(tomli query --filepath Cargo.toml workspace.package.publish 2>/dev/null || tomli query --filepath Cargo.toml package.publish 2>/dev/null)" == "" ] ; then \
		echo "{{error(['The', BOLD + 'publish', 'key is not set in', BOLD + 'Cargo.toml'])}}" && exit 1; fi

	echo "{{dim('publish = true')}}"
	if [ -n "$(tomli query --filepath Cargo.toml workspace.package.publish 2>/dev/null || tomli query --filepath Cargo.toml package.publish 2>/dev/null)" ] ; then \
		echo "{{error(['The', BOLD + 'publish', 'key is not set to', BOLD + 'true'])}}" && exit 1; fi

	echo {{CYAN}}Run {{YELLOW}}cargo publish{{CYAN}} in {{GREEN}}dry-run{{NORMAL}}
	cargo publish --dry-run --all-features --workspace --locked --allow-dirty

	echo {{done('')}}


# Publish the crate to crates.io
[group('Cargo')]
publish: publish_check
	echo {{CYAN}}Publish to {{GREEN}}crates.io{{NORMAL}}
	cargo publish
	echo {{done('')}}
