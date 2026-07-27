#  .just/github.just

# Requirements:
# zizmor
# - `cargo install --locked zizmor`
# pinact:
# - macOS: `brew install pinact`
# - Windows: `winget install --id suzuki-shunsuke.pinact`

# Variables.
# Return the repository name.
repo := shell("gh repo view --json name --jq '.name'")
# Return the repository owner (organization).
owner := shell("gh repo view --json owner --jq '.owner.login'")
# Return the GitHub user name.
user := shell("gh api user --jq '.login'")


# Audit GitHub Actions with `zizmor`
[group('GitHub Actions')]
[no-exit-message]
audit_actions:
	echo {{CYAN}}Audit actions in {{GREEN}}.github/workflows/*.yaml{{NORMAL}}
	zizmor --persona=auditor .
	echo {{done('')}}


# Update and pin GitHub Actions versions with `pinact` with cooldown
[group('GitHub Actions')]
[no-exit-message]
update_actions:
	echo {{CYAN}}Update and pin actions in {{YELLOW}}.github/workflows/*.yaml{{NORMAL}}
	pinact run --update --verify-comment --min-age 7
	echo {{done('')}}


# Update and pin GitHub Actions versions with `pinact`
[group('GitHub Actions')]
[private]
update_actions_latest:
	echo {{CYAN}}Update and pin actions in {{YELLOW}}.github/workflows/*.yaml{{NORMAL}}
	pinact run --update --verify-comment
	echo {{done('')}}


# Check GitHub Actions versions with `pinact`
[group('GitHub Actions')]
check_actions:
	echo {{CYAN}}Check actions in {{YELLOW}}.github/workflows/*.yaml{{NORMAL}}
	pinact run --verify-comment
	echo {{done('')}}
