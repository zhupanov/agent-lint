#!/usr/bin/env bash
# gh pr create --body "$BODY"
gh pr create --body 'static body'
gh pr create --body "$BODY" --body-file body.md
gh secret set TOKEN --body "$TOKEN"
gh variable set NAME --body "$VALUE"
gh project item-create 1 --body "$BODY"
printf '%s\n' 'gh pr create --body "$BODY"'
