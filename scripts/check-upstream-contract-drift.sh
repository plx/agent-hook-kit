#!/bin/sh
set -eu

codex_pinned=9e552e9d15ba52bed7077d5357f3e18e330f8f38
gemini_pinned=f354eebaf43b25bacb176007e449bb9a638fd101

codex_current=$(git ls-remote https://github.com/openai/codex.git HEAD | awk '{print $1}')
gemini_current=$(git ls-remote https://github.com/google-gemini/gemini-cli.git HEAD | awk '{print $1}')

printf 'Codex pinned=%s current=%s drift=%s\n' "$codex_pinned" "$codex_current" "$([ "$codex_pinned" = "$codex_current" ] && printf no || printf yes)"
printf 'Gemini pinned=%s current=%s drift=%s\n' "$gemini_pinned" "$gemini_current" "$([ "$gemini_pinned" = "$gemini_current" ] && printf no || printf yes)"
printf 'Mutable Claude and Antigravity documentation requires a reviewed dated capture; see contracts/MAINTENANCE.md.\n'
