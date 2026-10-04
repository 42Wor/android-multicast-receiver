#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

git filter-branch -f \
  --msg-filter 'sed "/[Cc]o-authored-by/d"' \
  --tag-name-filter cat \
  -- --all

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
