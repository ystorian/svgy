# .just/git.just


# Variables.
main_branch := 'main'


# Install the git hooks from `.githooks`
[group('Git')]
install_git_hooks:
	# Run this once per clone.
	echo {{CYAN}}Install git hooks from {{GREEN}}.githooks{{NORMAL}}
	git config core.hooksPath .githooks
	echo {{done('')}}


# Check if this release can be tagged `v#.#.#`
[group('Git')]
[no-exit-message]
check_tag tag=tag:
	echo {{CYAN}}Check if this release can be tagged {{GREEN + tag + NORMAL}}

	echo "{{dim('Check tag starting with vx.x.x')}}"
	if [ "{{replace_regex(tag, '^v\d.*', '')}}" != "" ]; then \
		echo "{{error(['Tag name must start with', GREEN + 'v', 'not', RED + replace_regex(tag, '\d+\.\d+\.\d+.*', '')])}}" && exit 1; fi

	echo "{{dim('Check semver format')}}"
	if [ "{{replace_regex(tag, 'v\d+\.\d+\.\d+', '')}}" != "" ]; then \
		echo "{{error(['Tag version is not a', BOLD + 'semver', 'version like', GREEN + 'x.y.z'])}}" && exit 1; fi

	echo "{{dim('Check version')}}"
	if [ "{{semver_matches(replace(tag, 'v', ''), '>=' + version)}}" != "true" ]; then \
		echo "{{error(['Tag version must be equal or higher than', GREEN + version, 'not', RED + replace(tag, 'v', '')])}}" && exit 1; fi

	echo "{{dim('Check working tree status')}}"
	if [ -n "$(git status --porcelain)" ]; then \
		echo "{{error(['The working tree is dirty', ITALIC + "\n" + shell('git status --porcelain')])}}" && exit 1; fi

	echo "{{dim('Check branch name')}}"
	if [ "$(git branch --show-current)" != "{{main_branch}}" ]; then \
		echo "{{error(['Not on the', GREEN + main_branch, 'branch:', RED + shell('git branch --show-current')])}}" && exit 1; fi

	echo "{{dim('Check remote branch')}}"
	git fetch --quiet origin {{main_branch}}
	if [ "$(git rev-parse {{main_branch}})" != "$(git rev-parse origin/{{main_branch}})" ]; then \
		echo "{{error(['The local', BOLD + main_branch, 'branch differs from', BOLD + 'origin/' + main_branch])}}" && exit 1; fi

	echo "{{dim('Check remote tag')}}"
	if [ "$(git rev-parse --verify --quiet "refs/tags/{{tag}}" >/dev/null)" ]; then \
		echo "{{error(['The tag', BOLD + tag, 'already exists'])}}" && exit 1; fi

	echo {{done('')}}

# Push an annotated release tag `v#.#.#`
[group('Git')]
push_tag tag=tag commit='':
	just check_tag {{tag}}
	echo {{CYAN}}Push the tag {{GREEN + tag + NORMAL}}
	git tag --sign {{tag}} {{commit}} --message "Release {{tag}}"
	git push origin refs/tags/{{tag}}
	echo {{done('')}}
