#!/bin/bash
# g++ wrapper that pretends to be g++-11
exec /usr/bin/g++-10 -D__GNUC__=11 -D__GNUC_MINOR__=0 -D__GNUC_PATCHLEVEL__=0 "$@"