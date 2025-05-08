#!/bin/bash
echo "output path: $1"
echo "golden path: $2"
echo "epsilon: $epsilon"
output=$(cat $1)
golden=$(cat $2)

if [[ "$output" != "$golden" ]]; then
    echo "assert output==golden failed" >&2
    exit 1
fi