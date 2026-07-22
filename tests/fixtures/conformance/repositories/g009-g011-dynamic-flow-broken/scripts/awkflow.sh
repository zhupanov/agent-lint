#!/usr/bin/env bash
awk 'BEGIN { re="—"; if ($0 ~ re) print "match" }'
