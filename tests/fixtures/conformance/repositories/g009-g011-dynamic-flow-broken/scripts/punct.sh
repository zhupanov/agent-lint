#!/usr/bin/env bash
set -u
items=()
sep=${sep%;}
printf '%s\n' "${items[@]}" "${PATH//:/;}"
