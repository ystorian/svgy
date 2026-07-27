# justfile

# Set the default shell on Windows to `bash` (installed with Git).
set windows-shell := ['C:\Program Files\Git\bin\bash.exe', '-cu']

# Don't print comments.
set ignore-comments

# Allow variable redefinition.
set allow-duplicate-variables

# Be quiet.
set quiet

# Enable new features.
set unstable
set lists

# Skip evaluating unused variables.
set lazy


# Project specifics.
# Load variables declared in `.env`.
set dotenv-load


# Project specifics.
# Variables.
# Language recipes.
import '.just/cargo.just'
import '.just/rust.just'


# Common variables, functions, and recipes.

# Variables.
# The default `main` branch for all repos.
main_branch := 'main'

# Architecture, using normalized output from `arch()`.
# - x86_64  -> x64
# - aarch64 -> arm64
arch_short := if arch() == "x86_64" { "x64" } else if arch() == "aarch64" { "arm64" } else { arch() }


# Functions.

# Display after a successful recipe.
# ✅ 2026-07-15 12:07:16.571 <recipe name> [<recipe arguments>]
done(args) := f'{{GREEN}}✅ {{style('dim') + datetime('%F %T%.3f ') + NORMAL + GREEN +
	BOLD + recipe_name() + NORMAL + GREEN + ITALIC}} {{join_list(args) + NORMAL + "\n"}}'

# Display an error message.
# Note: add `[no-exit-message]` on recipes for cleaner output.
# ❌ 2026-07-15 12:28:39.334 <recipe name>
# Error: [<message>]
error(message) := f'{{RED}}❌ {{style('dim') + recipe_name() + NORMAL + RED + BOLD + ' Error: ' +
	NORMAL + YELLOW + join_list(message, ' ' + NORMAL + YELLOW) + NORMAL + "\n"}}'

# Display a dimmed and indented message.
# __[<message>]
dim(message) := f'{{style('dim') + CYAN + '  ' + join_list(message, ' ' + NORMAL + style('dim')) + NORMAL}}'

# Get a file size.
file_size(file) := trim(shell('wc -c < "$1"', file))


# Import common recipes
import '.just/ci.just'
import '.just/git.just'
import '.just/github.just'


# Utility recipes.

# List the commands when called without parameters.
_:
	just --list


# Create an empty file
[group('Utils')]
[private]
create_file file:
	echo "{{dim(GREEN + 'Create empty file ' + BOLD + file)}}"
	rm -f "{{file}}" 2>/dev/null || true
	touch "{{file}}"


# Delete a directory
[group('Utils')]
[private]
delete_dir dir:
	if [ -d "{{dir}}" ]; then \
		echo "{{dim(YELLOW + 'Delete directory ' + BOLD + dir)}}" && \
		rm -rf "{{dir}}" 2>/dev/null || true ;fi


# Delete a file
[group('Utils')]
[private]
delete_file file:
	if [ -f "{{file}}" ]; then \
		echo "{{dim(YELLOW + 'Delete file ' + BOLD + file)}}" && \
		rm -f "{{file}}" 2>/dev/null || true ;fi


# Rename a file
[group('Utils')]
[private]
rename_file old_file new_file:
	echo "{{dim([YELLOW + 'Rename', YELLOW + BOLD + old_file, YELLOW + 'to', GREEN + BOLD + new_file])}}"
	mv "{{old_file}}" "{{new_file}}"


# Test if an environment variable is set
[group('Utils')]
[private]
[no-exit-message]
test_var variable:
	if [ -z "$(printenv {{variable}})" ]; then \
		echo "{{error(['The environment variable', BOLD + variable, 'is not declared', ITALIC + '(case-sensitive)'])}}" && exit 1; fi
