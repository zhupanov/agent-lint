#!/bin/sh
gh issue create -b "$ISSUE_BODY"
gh pr review 42 --body="$REVIEW_BODY"
gh discussion comment 7 --body "first
second"
