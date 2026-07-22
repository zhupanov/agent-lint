#!/usr/bin/env bash
set -u
arr=(safe)
(arr=())
printf '%s\n' "${arr[@]}"
(
  arr=()
  true )
printf '%s\n' "${arr[@]}"
