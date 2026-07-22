#!/bin/sh
gh secret set TOKEN --body "$TOKEN"
gh variable set NAME --body "$VALUE"
gh project item-create 1 --body "$BODY"
printf '%s\n' 'gh pr create --body "$BODY"'
weigh --body 5
sleigh --notes x
