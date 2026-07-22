#!/bin/sh
gh issue create --body 'short static body'
gh pr create --body-file body.md
gh release create v1.0.0 --notes-file -
