#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

# Use a portable msg filter (avoid GNU sed -e quoting issues under MSYS/Git Bash).
msg_filter() {
  python -c 'import sys; sys.stdout.write("".join(l for l in sys.stdin if "o-authored-by" not in l.lower()))'
}

export FILTER_BRANCH_SQUELCH_WARNING=1
git filter-branch -f \
  --msg-filter 'python -c "import sys; sys.stdout.write(\"\".join(l for l in sys.stdin if \"o-authored-by\" not in l.lower()))"' \
  --tag-name-filter cat \
  -- --all

rm -rf .git/refs/original/
git reflog expire --expire=now --all
git gc --prune=now

echo "FILTER_OK"
echo "--- recent commits ---"
git log --format=full -6
echo "--- co-authored check ---"
if git log --all --format=%B | grep -i 'co-authored-by'; then
  echo "STILL_HAS_COAUTHOR"
  exit 1
else
  echo "CLEAN_NO_COAUTHOR"
fi
