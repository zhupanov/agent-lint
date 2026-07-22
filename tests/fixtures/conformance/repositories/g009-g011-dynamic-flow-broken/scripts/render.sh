#!/usr/bin/env bash
replacement='&'
out=${text/TOKEN/`printf '%s' "$replacement"`}
out=${text//TOKEN/`printf '%s' "$replacement"`}
