#!/usr/bin/env bash
# Generates the API schema the checks run against.
mkdir -p .gen && printf '{"version": 3}\n' >.gen/schema.json && echo "schema generated"
