#!/usr/bin/env bash
replacement='&'
quoted=${text//TOKEN/"`printf '%s' x`"}
literal=${text//TOKEN/'`cmd`'}
escaped=${text//TOKEN/\`}
# commented=${text//TOKEN/`cmd`}
printf '%s\n' 'inert=${text//TOKEN/`cmd`}'
