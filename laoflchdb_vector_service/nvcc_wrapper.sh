#!/bin/bash
echo "NVCC_WRAPPER: called with $@" >> /tmp/nvcc_wrapper.log
# Add -allow-unsupported-compiler and pass through
exec /usr/local/cuda-11.8/bin/nvcc -allow-unsupported-compiler "$@"